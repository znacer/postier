use std::{
    collections::HashMap,
    fs::File,
    io::{BufReader, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

use anyhow::{Context, Result, bail};

use crate::model::{AssignedLabel, EmailRecord, LabelDefinition};

#[allow(dead_code)]
pub const HF_LAYA_REPO: &str = "https://huggingface.co/ggml-org/Laya-GGUF";
pub const HF_LAYA_Q8_URL: &str =
    "https://huggingface.co/ggml-org/Laya-GGUF/resolve/main/Laya-Q8_0.gguf";
pub const DEFAULT_LAYA_FILENAME: &str = "Laya-Q8_0.gguf";

const GGUF_MAGIC: &[u8; 4] = b"GGUF";
const GGML_TYPE_F32: u32 = 0;
const GGML_TYPE_F16: u32 = 1;
const GGML_TYPE_Q8_0: u32 = 8;
const GGML_TYPE_BF16: u32 = 30;

type CachedLayaModel = (PathBuf, Arc<LayaModel>);
type GgufParsed = (
    HashMap<String, GgufValue>,
    HashMap<String, TensorInfo>,
    usize,
);

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuestionType {
    Choice = 0,
    Score = 1,
    Noul = 2,
}

impl QuestionType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Choice => "choice",
            Self::Score => "score",
            Self::Noul => "noul",
        }
    }
}

#[derive(Clone, Debug)]
pub struct DecisionAnswer {
    pub choice: String,
    pub probabilities: Vec<(String, f64)>,
    pub confidence: f64,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
enum GgufValue {
    U32(u32),
    I32(i32),
    U64(u64),
    I64(i64),
    F32(f32),
    F64(f64),
    Bool(bool),
    String(String),
    Array(Vec<GgufValue>),
}

impl GgufValue {
    fn as_u32(&self) -> Option<u32> {
        match self {
            Self::U32(v) => Some(*v),
            Self::I32(v) => (*v >= 0).then_some(*v as u32),
            Self::U64(v) => Some(*v as u32),
            Self::I64(v) => (*v >= 0).then_some(*v as u32),
            _ => None,
        }
    }

    fn as_f32(&self) -> Option<f32> {
        match self {
            Self::F32(v) => Some(*v),
            Self::F64(v) => Some(*v as f32),
            Self::U32(v) => Some(*v as f32),
            Self::I32(v) => Some(*v as f32),
            _ => None,
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    fn as_u32_vec(&self) -> Option<Vec<u32>> {
        match self {
            Self::Array(items) => items.iter().map(Self::as_u32).collect(),
            _ => None,
        }
    }

    fn as_string_vec(&self) -> Option<Vec<String>> {
        match self {
            Self::Array(items) => items
                .iter()
                .map(|v| v.as_str().map(str::to_owned))
                .collect(),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
struct TensorInfo {
    dims: Vec<usize>,
    ggml_type: u32,
    offset: usize,
    byte_len: usize,
}

#[derive(Clone)]
enum TensorStorage {
    F32(Vec<f32>),
    BF16(Vec<u16>),
    F16(Vec<u16>),
    Q8_0(Vec<u8>),
}

#[derive(Clone)]
struct WeightTensor {
    ne0: usize,
    ne1: usize,
    storage: TensorStorage,
}

impl WeightTensor {
    fn get_row(&self, row: usize, out: &mut [f32]) {
        debug_assert_eq!(out.len(), self.ne0);
        let start = row * self.ne0;
        match &self.storage {
            TensorStorage::F32(data) => {
                out.copy_from_slice(&data[start..start + self.ne0]);
            }
            TensorStorage::BF16(data) => {
                for (dst, &raw) in out.iter_mut().zip(&data[start..start + self.ne0]) {
                    *dst = bf16_to_f32(raw);
                }
            }
            TensorStorage::F16(data) => {
                for (dst, &raw) in out.iter_mut().zip(&data[start..start + self.ne0]) {
                    *dst = f16_to_f32(raw);
                }
            }
            TensorStorage::Q8_0(bytes) => {
                let blocks_per_row = self.ne0 / 32;
                let row_byte_start = row * blocks_per_row * 34;
                for b in 0..blocks_per_row {
                    let block = &bytes[row_byte_start + b * 34..row_byte_start + (b + 1) * 34];
                    let scale = f16_to_f32(u16::from_le_bytes([block[0], block[1]]));
                    let dst_slice = &mut out[b * 32..(b + 1) * 32];
                    for i in 0..32 {
                        dst_slice[i] = scale * (block[2 + i] as i8 as f32);
                    }
                }
            }
        }
    }

    fn dot_row(&self, row: usize, input: &[f32]) -> f32 {
        debug_assert_eq!(input.len(), self.ne0);
        let start = row * self.ne0;
        match &self.storage {
            TensorStorage::F32(data) => data[start..start + self.ne0]
                .iter()
                .zip(input)
                .map(|(w, x)| w * x)
                .sum(),
            TensorStorage::BF16(data) => data[start..start + self.ne0]
                .iter()
                .zip(input)
                .map(|(&w, &x)| bf16_to_f32(w) * x)
                .sum(),
            TensorStorage::F16(data) => data[start..start + self.ne0]
                .iter()
                .zip(input)
                .map(|(&w, &x)| f16_to_f32(w) * x)
                .sum(),
            TensorStorage::Q8_0(bytes) => {
                let blocks_per_row = self.ne0 / 32;
                let row_byte_start = row * blocks_per_row * 34;
                let mut total = 0.0f32;
                for b in 0..blocks_per_row {
                    let block = &bytes[row_byte_start + b * 34..row_byte_start + (b + 1) * 34];
                    let scale = f16_to_f32(u16::from_le_bytes([block[0], block[1]]));
                    let x_slice = &input[b * 32..(b + 1) * 32];
                    let mut block_sum = 0.0f32;
                    for i in 0..32 {
                        block_sum += (block[2 + i] as i8 as f32) * x_slice[i];
                    }
                    total += scale * block_sum;
                }
                total
            }
        }
    }

    fn linear(&self, input: &[f32], bias: Option<&[f32]>, out: &mut [f32]) {
        debug_assert_eq!(input.len(), self.ne0);
        debug_assert_eq!(out.len(), self.ne1);
        for (row, dst) in out.iter_mut().enumerate() {
            let b = bias.map_or(0.0, |b| b[row]);
            *dst = self.dot_row(row, input) + b;
        }
    }
}

#[derive(Clone)]
struct EncoderLayer {
    attn_norm: Option<Vec<f32>>,
    wqkv: WeightTensor,
    wo: WeightTensor,
    ffn_norm: Vec<f32>,
    ffn_up: WeightTensor,
    ffn_down: WeightTensor,
    n_ff: usize,
}

#[derive(Clone)]
struct DecisionHeadLayer {
    attn_norm: Vec<f32>,
    attn_norm_b: Vec<f32>,
    wqkv: WeightTensor,
    wqkv_b: Vec<f32>,
    wo: WeightTensor,
    wo_b: Vec<f32>,
    ffn_norm: Vec<f32>,
    ffn_norm_b: Vec<f32>,
    ffn_up: WeightTensor,
    ffn_up_b: Vec<f32>,
    ffn_down: WeightTensor,
    ffn_down_b: Vec<f32>,
    n_ff: usize,
}

#[derive(Clone)]
struct BpeTokenizer {
    token_to_id: HashMap<String, u32>,
    bpe_ranks: HashMap<(String, String), usize>,
    byte_encoder: [char; 256],
    cls_id: u32,
    sep_id: u32,
    mask_id: u32,
    unk_id: u32,
}

impl BpeTokenizer {
    fn from_gguf_meta(meta: &HashMap<String, GgufValue>) -> Result<Self> {
        let tokens = meta
            .get("tokenizer.ggml.tokens")
            .and_then(GgufValue::as_string_vec)
            .context("GGUF missing tokenizer.ggml.tokens")?;
        let merges = meta
            .get("tokenizer.ggml.merges")
            .and_then(GgufValue::as_string_vec)
            .unwrap_or_default();
        let mut token_to_id = HashMap::with_capacity(tokens.len());
        for (idx, tok) in tokens.into_iter().enumerate() {
            token_to_id.insert(tok, idx as u32);
        }
        let mut bpe_ranks = HashMap::with_capacity(merges.len());
        for (rank, merge) in merges.into_iter().enumerate() {
            if let Some((a, b)) = merge.split_once(' ') {
                bpe_ranks.insert((a.to_owned(), b.to_owned()), rank);
            }
        }
        let cls_id = meta
            .get("tokenizer.ggml.bos_token_id")
            .and_then(GgufValue::as_u32)
            .or_else(|| token_to_id.get("[CLS]").copied())
            .unwrap_or(0);
        let sep_id = meta
            .get("tokenizer.ggml.seperator_token_id")
            .or_else(|| meta.get("tokenizer.ggml.eos_token_id"))
            .and_then(GgufValue::as_u32)
            .or_else(|| token_to_id.get("[SEP]").copied())
            .unwrap_or(0);
        let mask_id = meta
            .get("tokenizer.ggml.mask_token_id")
            .and_then(GgufValue::as_u32)
            .or_else(|| token_to_id.get("[MASK]").copied())
            .unwrap_or(0);
        let unk_id = meta
            .get("tokenizer.ggml.unknown_token_id")
            .and_then(GgufValue::as_u32)
            .or_else(|| token_to_id.get("[UNK]").copied())
            .unwrap_or(0);

        Ok(Self {
            token_to_id,
            bpe_ranks,
            byte_encoder: gpt2_byte_encoder(),
            cls_id,
            sep_id,
            mask_id,
            unk_id,
        })
    }

    fn encode(&self, text: &str) -> Vec<u32> {
        let sanitized = text.replace("[MASK]", " ");
        if sanitized.is_empty() {
            return Vec::new();
        }
        let mut ids = Vec::new();
        for piece in split_gpt2_words(&sanitized) {
            if let Some(&direct) = self.token_to_id.get(piece) {
                ids.push(direct);
                continue;
            }
            let encoded: String = piece
                .as_bytes()
                .iter()
                .map(|&b| self.byte_encoder[b as usize])
                .collect();
            for bpe_tok in self.bpe(&encoded) {
                if let Some(&id) = self.token_to_id.get(&bpe_tok) {
                    ids.push(id);
                } else if let Some(&id) = self.token_to_id.get(&bpe_tok.to_ascii_lowercase()) {
                    ids.push(id);
                } else {
                    ids.push(self.unk_id);
                }
            }
        }
        ids
    }

    fn bpe(&self, token: &str) -> Vec<String> {
        if self.bpe_ranks.is_empty() {
            return vec![token.to_owned()];
        }
        let mut word: Vec<String> = token.chars().map(|c| c.to_string()).collect();
        if word.len() <= 1 {
            return word;
        }
        loop {
            let mut best: Option<(usize, usize)> = None;
            for i in 0..word.len().saturating_sub(1) {
                let pair = (word[i].clone(), word[i + 1].clone());
                if let Some(&rank) = self.bpe_ranks.get(&pair)
                    && best.is_none_or(|(best_rank, _)| rank < best_rank)
                {
                    best = Some((rank, i));
                }
            }
            let Some((_, idx)) = best else {
                break;
            };
            let first = word[idx].clone();
            let second = word[idx + 1].clone();
            let merged = format!("{first}{second}");
            let mut next = Vec::with_capacity(word.len());
            let mut i = 0;
            while i < word.len() {
                if i + 1 < word.len() && word[i] == first && word[i + 1] == second {
                    next.push(merged.clone());
                    i += 2;
                } else {
                    next.push(word[i].clone());
                    i += 1;
                }
            }
            word = next;
            if word.len() == 1 {
                break;
            }
        }
        word
    }
}

#[allow(dead_code)]
#[derive(Clone)]
pub struct LayaModel {
    pub name: String,
    pub is_embedded: bool,
    n_embd: usize,
    n_head: usize,
    rope_freq_base: f32,
    rope_freq_base_swa: f32,
    sliding_window: usize,
    sliding_window_pattern: usize,
    norm_eps: f32,
    max_len: usize,
    max_head_tokens: usize,
    temperatures: HashMap<String, f32>,
    tokenizer: BpeTokenizer,
    tok_embd: WeightTensor,
    tok_norm: Vec<f32>,
    encoder_layers: Vec<EncoderLayer>,
    output_norm: Vec<f32>,
    type_embd: Vec<f32>,
    head_layers: Vec<DecisionHeadLayer>,
    cls_norm: Vec<f32>,
    cls_norm_b: Vec<f32>,
    cls_weight: WeightTensor,
    cls_bias: Vec<f32>,
    cls_out_weight: WeightTensor,
    cls_out_bias: f32,
}

static MODEL_CACHE: OnceLock<Mutex<Option<CachedLayaModel>>> = OnceLock::new();
static EMBEDDED_MODEL: OnceLock<Arc<LayaModel>> = OnceLock::new();

impl LayaModel {
    pub fn load_or_embedded(gguf_path: Option<&Path>) -> Result<Arc<Self>> {
        if let Some(path) = gguf_path.filter(|p| p.is_file()) {
            let cache = MODEL_CACHE.get_or_init(|| Mutex::new(None));
            if let Ok(guard) = cache.lock()
                && let Some((cached_path, model)) = guard.as_ref()
                && cached_path == path
            {
                return Ok(model.clone());
            }
            let loaded = Arc::new(Self::from_file(path)?);
            if let Ok(mut guard) = cache.lock() {
                *guard = Some((path.to_path_buf(), loaded.clone()));
            }
            return Ok(loaded);
        }
        Ok(Self::embedded())
    }

    pub fn embedded() -> Arc<Self> {
        EMBEDDED_MODEL
            .get_or_init(|| {
                let bytes = build_packaged_laya_gguf();
                Arc::new(
                    Self::from_bytes(&bytes, true)
                        .expect("packaged Laya-GGUF decision model must be valid"),
                )
            })
            .clone()
    }

    pub fn from_file(path: &Path) -> Result<Self> {
        let mut file =
            BufReader::new(File::open(path).with_context(|| format!("open {}", path.display()))?);
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .with_context(|| format!("read {}", path.display()))?;
        Self::from_bytes(&bytes, false)
    }

    pub fn from_bytes(bytes: &[u8], is_embedded: bool) -> Result<Self> {
        let (meta, tensors, data_offset) = parse_gguf(bytes)?;
        let arch = meta
            .get("general.architecture")
            .and_then(GgufValue::as_str)
            .unwrap_or("modern-bert");
        if arch != "modern-bert" {
            bail!("unsupported GGUF architecture {arch:?}; expected \"modern-bert\"");
        }
        let decision_type = meta
            .get("modern-bert.decision.type")
            .and_then(GgufValue::as_str)
            .unwrap_or("laya");
        if decision_type != "laya" {
            bail!("unsupported decision head {decision_type:?}; expected \"laya\"");
        }

        let name = meta
            .get("general.name")
            .and_then(GgufValue::as_str)
            .unwrap_or("Laya")
            .to_owned();
        let n_layer = meta
            .get("modern-bert.block_count")
            .and_then(GgufValue::as_u32)
            .context("missing modern-bert.block_count")? as usize;
        let n_layer_decision = meta
            .get("modern-bert.decision.block_count")
            .and_then(GgufValue::as_u32)
            .unwrap_or(2) as usize;
        if n_layer <= n_layer_decision {
            bail!("invalid block counts: total={n_layer}, decision={n_layer_decision}");
        }
        let n_layer_enc = n_layer - n_layer_decision;
        let n_embd = meta
            .get("modern-bert.embedding_length")
            .and_then(GgufValue::as_u32)
            .context("missing modern-bert.embedding_length")? as usize;
        let n_head = meta
            .get("modern-bert.attention.head_count")
            .and_then(GgufValue::as_u32)
            .context("missing modern-bert.attention.head_count")? as usize;
        if n_head == 0 || n_embd == 0 || !n_embd.is_multiple_of(n_head) {
            bail!("invalid embedding/head dimensions: n_embd={n_embd}, n_head={n_head}");
        }
        let ff_lengths = meta
            .get("modern-bert.feed_forward_length")
            .and_then(GgufValue::as_u32_vec)
            .map(|v| v.into_iter().map(|x| x as usize).collect::<Vec<_>>())
            .unwrap_or_else(|| vec![n_embd * 2; n_layer]);
        let rope_freq_base = meta
            .get("modern-bert.rope.freq_base")
            .and_then(GgufValue::as_f32)
            .unwrap_or(160_000.0);
        let rope_freq_base_swa = meta
            .get("modern-bert.rope.freq_base_swa")
            .and_then(GgufValue::as_f32)
            .unwrap_or(10_000.0);
        let sliding_window = meta
            .get("modern-bert.attention.sliding_window")
            .and_then(GgufValue::as_u32)
            .unwrap_or(128) as usize;
        let sliding_window_pattern = meta
            .get("modern-bert.attention.sliding_window_pattern")
            .and_then(GgufValue::as_u32)
            .unwrap_or(3) as usize;
        let norm_eps = meta
            .get("modern-bert.attention.layer_norm_epsilon")
            .and_then(GgufValue::as_f32)
            .unwrap_or(1e-5);
        let max_head_tokens = meta
            .get("modern-bert.decision.max_head_tokens")
            .and_then(GgufValue::as_u32)
            .unwrap_or(384) as usize;

        let mut temperatures = HashMap::new();
        for (k, v) in &meta {
            if let Some(suffix) = k.strip_prefix("modern-bert.decision.temperature.")
                && let Some(temp) = v.as_f32()
            {
                temperatures.insert(suffix.to_owned(), temp);
            }
        }

        let tokenizer = BpeTokenizer::from_gguf_meta(&meta)?;

        let load_vec = |tensor_name: &str| -> Result<Vec<f32>> {
            let wt = load_weight_tensor(bytes, data_offset, &tensors, tensor_name)?;
            let mut out = vec![0.0f32; wt.ne0 * wt.ne1];
            for r in 0..wt.ne1 {
                wt.get_row(r, &mut out[r * wt.ne0..(r + 1) * wt.ne0]);
            }
            Ok(out)
        };

        let tok_embd = load_weight_tensor(bytes, data_offset, &tensors, "token_embd.weight")?;
        let tok_norm = load_vec("token_embd_norm.weight")?;
        let output_norm = load_vec("output_norm.weight")?;
        let type_embd = load_vec("token_types.weight")?;
        if type_embd.len() < 3 * n_embd {
            bail!(
                "token_types.weight length {} is smaller than {}",
                type_embd.len(),
                3 * n_embd
            );
        }

        let mut encoder_layers = Vec::with_capacity(n_layer_enc);
        for il in 0..n_layer_enc {
            let attn_norm = if il == 0 {
                tensors
                    .contains_key("blk.0.attn_norm.weight")
                    .then(|| load_vec("blk.0.attn_norm.weight"))
                    .transpose()?
            } else {
                Some(load_vec(&format!("blk.{il}.attn_norm.weight"))?)
            };
            let wqkv = load_weight_tensor(
                bytes,
                data_offset,
                &tensors,
                &format!("blk.{il}.attn_qkv.weight"),
            )?;
            let wo = load_weight_tensor(
                bytes,
                data_offset,
                &tensors,
                &format!("blk.{il}.attn_output.weight"),
            )?;
            let ffn_norm = load_vec(&format!("blk.{il}.ffn_norm.weight"))?;
            let ffn_up = load_weight_tensor(
                bytes,
                data_offset,
                &tensors,
                &format!("blk.{il}.ffn_up.weight"),
            )?;
            let ffn_down = load_weight_tensor(
                bytes,
                data_offset,
                &tensors,
                &format!("blk.{il}.ffn_down.weight"),
            )?;
            let n_ff = ff_lengths.get(il).copied().unwrap_or(ffn_down.ne0);
            encoder_layers.push(EncoderLayer {
                attn_norm,
                wqkv,
                wo,
                ffn_norm,
                ffn_up,
                ffn_down,
                n_ff,
            });
        }

        let mut head_layers = Vec::with_capacity(n_layer_decision);
        for il in n_layer_enc..n_layer {
            let attn_norm = load_vec(&format!("blk.{il}.attn_norm.weight"))?;
            let attn_norm_b = load_vec(&format!("blk.{il}.attn_norm.bias"))?;
            let wqkv = load_weight_tensor(
                bytes,
                data_offset,
                &tensors,
                &format!("blk.{il}.attn_qkv.weight"),
            )?;
            let wqkv_b = load_vec(&format!("blk.{il}.attn_qkv.bias"))?;
            let wo = load_weight_tensor(
                bytes,
                data_offset,
                &tensors,
                &format!("blk.{il}.attn_output.weight"),
            )?;
            let wo_b = load_vec(&format!("blk.{il}.attn_output.bias"))?;
            let ffn_norm = load_vec(&format!("blk.{il}.ffn_norm.weight"))?;
            let ffn_norm_b = load_vec(&format!("blk.{il}.ffn_norm.bias"))?;
            let ffn_up = load_weight_tensor(
                bytes,
                data_offset,
                &tensors,
                &format!("blk.{il}.ffn_up.weight"),
            )?;
            let ffn_up_b = load_vec(&format!("blk.{il}.ffn_up.bias"))?;
            let ffn_down = load_weight_tensor(
                bytes,
                data_offset,
                &tensors,
                &format!("blk.{il}.ffn_down.weight"),
            )?;
            let ffn_down_b = load_vec(&format!("blk.{il}.ffn_down.bias"))?;
            let n_ff = ff_lengths.get(il).copied().unwrap_or(ffn_up.ne1);
            head_layers.push(DecisionHeadLayer {
                attn_norm,
                attn_norm_b,
                wqkv,
                wqkv_b,
                wo,
                wo_b,
                ffn_norm,
                ffn_norm_b,
                ffn_up,
                ffn_up_b,
                ffn_down,
                ffn_down_b,
                n_ff,
            });
        }

        let cls_norm = load_vec("cls.norm.weight")?;
        let cls_norm_b = load_vec("cls.norm.bias")?;
        let cls_weight = load_weight_tensor(bytes, data_offset, &tensors, "cls.weight")?;
        let cls_bias = load_vec("cls.bias")?;
        let cls_out_weight = load_weight_tensor(bytes, data_offset, &tensors, "cls.output.weight")?;
        let cls_out_bias = *load_vec("cls.output.bias")?.first().unwrap_or(&0.0);

        Ok(Self {
            name,
            is_embedded,
            n_embd,
            n_head,
            rope_freq_base,
            rope_freq_base_swa,
            sliding_window,
            sliding_window_pattern,
            norm_eps,
            max_len: 512,
            max_head_tokens,
            temperatures,
            tokenizer,
            tok_embd,
            tok_norm,
            encoder_layers,
            output_norm,
            type_embd,
            head_layers,
            cls_norm,
            cls_norm_b,
            cls_weight,
            cls_bias,
            cls_out_weight,
            cls_out_bias,
        })
    }

    pub fn predict_choice(
        &self,
        state: &str,
        instructions: &str,
        criteria: &[(String, String)],
    ) -> Result<DecisionAnswer> {
        if criteria.is_empty() {
            bail!("at least one label criterion is required");
        }
        let (tokens, markers) =
            self.build_laya_sequence(QuestionType::Choice, instructions, criteria, state);
        let raw_scores = self.forward_scores(&tokens, &markers, QuestionType::Choice)?;
        let temp = self.temperature_for(QuestionType::Choice, criteria.len());
        let max_score = raw_scores.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let mut exp_scores: Vec<f64> = raw_scores
            .iter()
            .map(|&s| (((s - max_score) / temp) as f64).exp())
            .collect();
        let sum: f64 = exp_scores.iter().sum::<f64>().max(1e-12);
        for p in &mut exp_scores {
            *p /= sum;
        }
        let mut best_idx = 0;
        for (i, &p) in exp_scores.iter().enumerate() {
            if p > exp_scores[best_idx] {
                best_idx = i;
            }
        }
        let confidence = if criteria.len() < 2 {
            1.0
        } else {
            let uniform = 1.0 / (criteria.len() as f64);
            ((exp_scores[best_idx] - uniform) / (1.0 - uniform)).clamp(0.0, 1.0)
        };
        let probabilities = criteria
            .iter()
            .map(|(k, _)| k.clone())
            .zip(exp_scores)
            .collect();
        Ok(DecisionAnswer {
            choice: criteria[best_idx].0.clone(),
            probabilities,
            confidence,
        })
    }

    pub fn label_email(
        &self,
        email: &EmailRecord,
        definitions: &[LabelDefinition],
    ) -> Result<Vec<AssignedLabel>> {
        if definitions.is_empty() {
            return Ok(Vec::new());
        }
        let state = format!(
            "Subject: {}\nFrom: {}\nTo: {}\nFolder: {}\nBody:\n{}",
            email.subject,
            email.sender,
            email.recipients,
            email.folder,
            email.excerpt(2_400)
        );
        let criteria: Vec<(String, String)> = definitions
            .iter()
            .map(|d| (d.name.clone(), d.description.clone()))
            .collect();
        let answer = self.predict_choice(
            &state,
            "Which label best categorizes this email?",
            &criteria,
        )?;
        let source_tag = if self.is_embedded {
            "laya-packaged"
        } else {
            "laya-gguf"
        };
        let mut assigned = Vec::new();
        let best_prob = answer
            .probabilities
            .iter()
            .find(|(k, _)| k == &answer.choice)
            .map(|(_, p)| *p)
            .unwrap_or(answer.confidence);
        assigned.push(AssignedLabel {
            label: answer.choice.clone(),
            confidence: Some(best_prob),
            source: source_tag.into(),
        });

        let secondary_threshold = (1.8 / (definitions.len().max(2) as f64)).max(0.28);
        for (label, prob) in &answer.probabilities {
            if label != &answer.choice && *prob >= secondary_threshold && assigned.len() < 2 {
                assigned.push(AssignedLabel {
                    label: label.clone(),
                    confidence: Some(*prob),
                    source: source_tag.into(),
                });
            }
        }
        Ok(assigned)
    }

    fn build_laya_sequence(
        &self,
        qtype: QuestionType,
        instructions: &str,
        options: &[(String, String)],
        state: &str,
    ) -> (Vec<u32>, Vec<usize>) {
        let mut head_ids =
            self.tokenizer
                .encode(&format!("{} question: {}", qtype.as_str(), instructions));
        let mut opt_spans: Vec<Vec<u32>> = options
            .iter()
            .map(|(k, desc)| {
                let text = if desc.trim().is_empty() {
                    format!(" {k}")
                } else {
                    format!(" {k}: {desc}")
                };
                let mut toks = self.tokenizer.encode(&text);
                toks.truncate(48);
                let mut span = Vec::with_capacity(1 + toks.len());
                span.push(self.tokenizer.mask_id);
                span.extend(toks);
                span
            })
            .collect();

        let total_opt_tokens: usize = opt_spans.iter().map(Vec::len).sum();
        if total_opt_tokens + 16 > self.max_head_tokens {
            let per = ((self.max_head_tokens.saturating_sub(16)) / options.len().max(1)).max(4);
            for span in &mut opt_spans {
                span.truncate(per);
            }
        }
        let used_opt_tokens: usize = opt_spans.iter().map(Vec::len).sum();
        let max_q = self.max_head_tokens.saturating_sub(used_opt_tokens).max(8);
        head_ids.truncate(max_q);

        let mut seq = Vec::with_capacity(self.max_len);
        seq.push(self.tokenizer.cls_id);
        seq.extend(head_ids);
        seq.push(self.tokenizer.sep_id);

        let mut markers = Vec::with_capacity(opt_spans.len());
        for span in opt_spans {
            markers.push(seq.len());
            seq.extend(span);
        }
        seq.push(self.tokenizer.sep_id);

        let room = self.max_len.saturating_sub(seq.len() + 1);
        let mut state_ids = self.tokenizer.encode(state);
        state_ids.truncate(room);
        seq.extend(state_ids);
        seq.push(self.tokenizer.sep_id);

        seq.truncate(self.max_len);
        markers.retain(|&m| m < seq.len());
        (seq, markers)
    }

    fn forward_scores(
        &self,
        tokens: &[u32],
        markers: &[usize],
        qtype: QuestionType,
    ) -> Result<Vec<f32>> {
        let seq_len = tokens.len();
        let d = self.n_embd;
        let head_dim = d / self.n_head;

        let mut hidden = vec![0.0f32; seq_len * d];
        let mut tmp_row = vec![0.0f32; d];
        for (pos, &tok) in tokens.iter().enumerate() {
            let row = (tok as usize).min(self.tok_embd.ne1.saturating_sub(1));
            self.tok_embd.get_row(row, &mut tmp_row);
            layer_norm(
                &tmp_row,
                &self.tok_norm,
                None,
                self.norm_eps,
                &mut hidden[pos * d..(pos + 1) * d],
            );
        }

        let mut normed = vec![0.0f32; seq_len * d];
        let mut qkv = vec![0.0f32; seq_len * 3 * d];
        let mut attn_out = vec![0.0f32; seq_len * d];
        let mut proj_out = vec![0.0f32; d];

        for (il, layer) in self.encoder_layers.iter().enumerate() {
            if let Some(attn_norm) = &layer.attn_norm {
                for pos in 0..seq_len {
                    layer_norm(
                        &hidden[pos * d..(pos + 1) * d],
                        attn_norm,
                        None,
                        self.norm_eps,
                        &mut normed[pos * d..(pos + 1) * d],
                    );
                }
            } else {
                normed.copy_from_slice(&hidden);
            }

            let is_global = self.sliding_window_pattern > 0
                && (il + 1).is_multiple_of(self.sliding_window_pattern);
            let freq_base = if is_global {
                self.rope_freq_base
            } else {
                self.rope_freq_base_swa
            };
            let max_dist =
                (!is_global && self.sliding_window > 0).then_some((self.sliding_window / 2).max(1));

            for pos in 0..seq_len {
                let in_slice = &normed[pos * d..(pos + 1) * d];
                let qkv_slice = &mut qkv[pos * 3 * d..(pos + 1) * 3 * d];
                layer.wqkv.linear(in_slice, None, qkv_slice);
                apply_rope_neox(&mut qkv_slice[0..d], self.n_head, head_dim, pos, freq_base);
                apply_rope_neox(
                    &mut qkv_slice[d..2 * d],
                    self.n_head,
                    head_dim,
                    pos,
                    freq_base,
                );
            }

            multihead_attention(
                &qkv,
                seq_len,
                d,
                self.n_head,
                head_dim,
                max_dist,
                &mut attn_out,
            );

            for pos in 0..seq_len {
                layer
                    .wo
                    .linear(&attn_out[pos * d..(pos + 1) * d], None, &mut proj_out);
                for i in 0..d {
                    hidden[pos * d + i] += proj_out[i];
                }
            }

            let mut ffn_buf = vec![0.0f32; 2 * layer.n_ff];
            let mut gated = vec![0.0f32; layer.n_ff];
            for pos in 0..seq_len {
                let h_slice = &mut hidden[pos * d..(pos + 1) * d];
                layer_norm(h_slice, &layer.ffn_norm, None, self.norm_eps, &mut tmp_row);
                layer.ffn_up.linear(&tmp_row, None, &mut ffn_buf);
                for i in 0..layer.n_ff {
                    gated[i] = gelu_erf(ffn_buf[i]) * ffn_buf[layer.n_ff + i];
                }
                layer.ffn_down.linear(&gated, None, &mut proj_out);
                for i in 0..d {
                    h_slice[i] += proj_out[i];
                }
            }
        }

        for pos in 0..seq_len {
            let h_slice = &hidden[pos * d..(pos + 1) * d];
            layer_norm(
                h_slice,
                &self.output_norm,
                None,
                self.norm_eps,
                &mut normed[pos * d..(pos + 1) * d],
            );
        }
        hidden.copy_from_slice(&normed);

        let type_idx = qtype as usize;
        let type_row = &self.type_embd[type_idx * d..(type_idx + 1) * d];
        for pos in 0..seq_len {
            for i in 0..d {
                hidden[pos * d + i] += type_row[i];
            }
        }

        for layer in &self.head_layers {
            for pos in 0..seq_len {
                layer_norm(
                    &hidden[pos * d..(pos + 1) * d],
                    &layer.attn_norm,
                    Some(&layer.attn_norm_b),
                    self.norm_eps,
                    &mut normed[pos * d..(pos + 1) * d],
                );
                layer.wqkv.linear(
                    &normed[pos * d..(pos + 1) * d],
                    Some(&layer.wqkv_b),
                    &mut qkv[pos * 3 * d..(pos + 1) * 3 * d],
                );
            }

            multihead_attention(&qkv, seq_len, d, self.n_head, head_dim, None, &mut attn_out);

            for pos in 0..seq_len {
                layer.wo.linear(
                    &attn_out[pos * d..(pos + 1) * d],
                    Some(&layer.wo_b),
                    &mut proj_out,
                );
                for i in 0..d {
                    hidden[pos * d + i] += proj_out[i];
                }
            }

            let mut up_buf = vec![0.0f32; layer.n_ff];
            for pos in 0..seq_len {
                let h_slice = &mut hidden[pos * d..(pos + 1) * d];
                layer_norm(
                    h_slice,
                    &layer.ffn_norm,
                    Some(&layer.ffn_norm_b),
                    self.norm_eps,
                    &mut tmp_row,
                );
                layer
                    .ffn_up
                    .linear(&tmp_row, Some(&layer.ffn_up_b), &mut up_buf);
                for v in &mut up_buf {
                    *v = v.max(0.0);
                }
                layer
                    .ffn_down
                    .linear(&up_buf, Some(&layer.ffn_down_b), &mut proj_out);
                for i in 0..d {
                    h_slice[i] += proj_out[i];
                }
            }
        }

        let mut scores = Vec::with_capacity(markers.len());
        let mut cls_hidden = vec![0.0f32; d];
        let mut out_scalar = [0.0f32; 1];
        for &m_pos in markers {
            let h_slice = &hidden[m_pos * d..(m_pos + 1) * d];
            layer_norm(
                h_slice,
                &self.cls_norm,
                Some(&self.cls_norm_b),
                self.norm_eps,
                &mut tmp_row,
            );
            self.cls_weight
                .linear(&tmp_row, Some(&self.cls_bias), &mut cls_hidden);
            for v in &mut cls_hidden {
                *v = gelu_erf(*v);
            }
            self.cls_out_weight
                .linear(&cls_hidden, None, &mut out_scalar);
            scores.push(out_scalar[0] + self.cls_out_bias);
        }
        Ok(scores)
    }

    fn temperature_for(&self, qtype: QuestionType, n_options: usize) -> f32 {
        let bucket = if n_options <= 2 {
            "2"
        } else if n_options <= 5 {
            "3_5"
        } else if n_options <= 10 {
            "6_10"
        } else {
            "11"
        };
        let type_name = qtype.as_str();
        let specific = format!("{type_name}.{bucket}");
        let raw = self
            .temperatures
            .get(&specific)
            .or_else(|| self.temperatures.get(type_name))
            .copied()
            .unwrap_or(1.0);
        raw.clamp(0.5, 5.0)
    }
}

pub fn default_gguf_path(storage_dir: &Path) -> PathBuf {
    let in_storage = storage_dir.join("models").join(DEFAULT_LAYA_FILENAME);
    if in_storage.is_file() {
        return in_storage;
    }
    let in_data = PathBuf::from("data").join(DEFAULT_LAYA_FILENAME);
    if in_data.is_file() {
        return in_data;
    }
    in_storage
}

pub fn download_laya_gguf(target_path: &Path) -> Result<u64> {
    if let Some(parent) = target_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("start download runtime")?;
    let tmp_path = target_path.with_extension("gguf.part");
    let total = runtime.block_on(async {
        let client = reqwest::Client::builder()
            .https_only(true)
            .build()
            .context("build HTTPS client for Laya-GGUF download")?;
        let mut response = client
            .get(HF_LAYA_Q8_URL)
            .send()
            .await
            .context("request Laya-Q8_0.gguf from Hugging Face")?
            .error_for_status()
            .context("Hugging Face returned an error for Laya-Q8_0.gguf")?;
        let mut out =
            File::create(&tmp_path).with_context(|| format!("create {}", tmp_path.display()))?;
        let mut written = 0u64;
        while let Some(chunk) = response
            .chunk()
            .await
            .context("read Laya-Q8_0.gguf stream chunk")?
        {
            out.write_all(&chunk)
                .context("write Laya-Q8_0.gguf chunk")?;
            written += chunk.len() as u64;
        }
        out.flush()?;
        Ok::<u64, anyhow::Error>(written)
    })?;
    std::fs::rename(&tmp_path, target_path)
        .with_context(|| format!("move downloaded model to {}", target_path.display()))?;
    Ok(total)
}

fn layer_norm(input: &[f32], weight: &[f32], bias: Option<&[f32]>, eps: f32, out: &mut [f32]) {
    let n = input.len() as f32;
    let mean = input.iter().sum::<f32>() / n;
    let var = input
        .iter()
        .map(|&x| {
            let diff = x - mean;
            diff * diff
        })
        .sum::<f32>()
        / n;
    let inv_std = 1.0 / (var + eps).sqrt();
    for i in 0..input.len() {
        let b = bias.map_or(0.0, |b| b[i]);
        out[i] = (input[i] - mean) * inv_std * weight[i] + b;
    }
}

fn apply_rope_neox(vec: &mut [f32], n_head: usize, head_dim: usize, pos: usize, freq_base: f32) {
    let half = head_dim / 2;
    for h in 0..n_head {
        let head = &mut vec[h * head_dim..(h + 1) * head_dim];
        for i in 0..half {
            let exponent = (2 * i) as f32 / (head_dim as f32);
            let theta = (pos as f32) * freq_base.powf(-exponent);
            let (sin_t, cos_t) = theta.sin_cos();
            let x0 = head[i];
            let x1 = head[i + half];
            head[i] = x0 * cos_t - x1 * sin_t;
            head[i + half] = x0 * sin_t + x1 * cos_t;
        }
    }
}

fn multihead_attention(
    qkv: &[f32],
    seq_len: usize,
    d: usize,
    n_head: usize,
    head_dim: usize,
    max_dist: Option<usize>,
    out: &mut [f32],
) {
    out.fill(0.0);
    let scale = 1.0 / (head_dim as f32).sqrt();
    let mut scores = vec![f32::NEG_INFINITY; seq_len];
    for h in 0..n_head {
        let h_off = h * head_dim;
        for qi in 0..seq_len {
            let q = &qkv[qi * 3 * d + h_off..qi * 3 * d + h_off + head_dim];
            let mut max_s = f32::NEG_INFINITY;
            for (ki, score_slot) in scores.iter_mut().enumerate() {
                if let Some(limit) = max_dist
                    && qi.abs_diff(ki) > limit
                {
                    *score_slot = f32::NEG_INFINITY;
                    continue;
                }
                let k = &qkv[ki * 3 * d + d + h_off..ki * 3 * d + d + h_off + head_dim];
                let dot: f32 = q.iter().zip(k).map(|(a, b)| a * b).sum();
                let s = dot * scale;
                *score_slot = s;
                if s > max_s {
                    max_s = s;
                }
            }
            let mut sum = 0.0f32;
            for s in &mut scores {
                if s.is_finite() {
                    *s = (*s - max_s).exp();
                    sum += *s;
                } else {
                    *s = 0.0;
                }
            }
            let inv_sum = if sum > 0.0 { 1.0 / sum } else { 0.0 };
            let out_head = &mut out[qi * d + h_off..qi * d + h_off + head_dim];
            for (ki, &weight) in scores.iter().enumerate() {
                let w = weight * inv_sum;
                if w == 0.0 {
                    continue;
                }
                let v = &qkv[ki * 3 * d + 2 * d + h_off..ki * 3 * d + 2 * d + h_off + head_dim];
                for j in 0..head_dim {
                    out_head[j] += w * v[j];
                }
            }
        }
    }
}

fn gelu_erf(x: f32) -> f32 {
    0.5 * x * (1.0 + erf_approx(x * std::f32::consts::FRAC_1_SQRT_2))
}

fn erf_approx(x: f32) -> f32 {
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let t = 1.0 / (1.0 + 0.3275911 * x);
    let y = 1.0
        - (((((1.0614054 * t - 1.453152) * t) + 1.4214137) * t - 0.28449674) * t + 0.2548296)
            * t
            * (-x * x).exp();
    sign * y
}

fn bf16_to_f32(bits: u16) -> f32 {
    f32::from_bits((bits as u32) << 16)
}

fn f16_to_f32(bits: u16) -> f32 {
    let sign = ((bits >> 15) as u32) << 31;
    let exp = ((bits >> 10) & 0x1f) as u32;
    let mant = (bits & 0x03ff) as u32;
    if exp == 0 {
        if mant == 0 {
            f32::from_bits(sign)
        } else {
            let mut m = mant;
            let mut e = 127u32 - 14;
            while (m & 0x0400) == 0 {
                m <<= 1;
                e -= 1;
            }
            m &= 0x03ff;
            f32::from_bits(sign | (e << 23) | (m << 13))
        }
    } else if exp == 31 {
        f32::from_bits(sign | 0x7f80_0000 | (mant << 13))
    } else {
        let e = exp + (127 - 15);
        f32::from_bits(sign | (e << 23) | (mant << 13))
    }
}

fn gpt2_byte_encoder() -> [char; 256] {
    let mut out = ['\0'; 256];
    let mut n = 0u32;
    for b in 0u16..=255 {
        let is_printable =
            (b'!' as u16..=b'~' as u16).contains(&b) || (0xA1..=0xAC).contains(&b) || b >= 0xAE;
        if is_printable {
            out[b as usize] = char::from_u32(b as u32).unwrap();
        } else {
            out[b as usize] = char::from_u32(256 + n).unwrap();
            n += 1;
        }
    }
    out
}

fn split_gpt2_words(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    while start < bytes.len() {
        let b = bytes[start];
        if b == b' ' && start + 1 < bytes.len() && bytes[start + 1] != b' ' {
            let mut end = start + 1;
            let first = bytes[end];
            if first.is_ascii_alphabetic() {
                while end < bytes.len() && bytes[end].is_ascii_alphabetic() {
                    end += 1;
                }
            } else if first.is_ascii_digit() {
                while end < bytes.len() && bytes[end].is_ascii_digit() {
                    end += 1;
                }
            } else {
                let ch_len = text[end..].chars().next().map_or(1, char::len_utf8);
                end += ch_len;
            }
            out.push(&text[start..end]);
            start = end;
        } else if b.is_ascii_alphabetic() {
            let mut end = start + 1;
            while end < bytes.len() && bytes[end].is_ascii_alphabetic() {
                end += 1;
            }
            out.push(&text[start..end]);
            start = end;
        } else if b.is_ascii_digit() {
            let mut end = start + 1;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            out.push(&text[start..end]);
            start = end;
        } else {
            let ch_len = text[start..].chars().next().map_or(1, char::len_utf8);
            out.push(&text[start..start + ch_len]);
            start += ch_len;
        }
    }
    out
}

fn parse_gguf(bytes: &[u8]) -> Result<GgufParsed> {
    if bytes.len() < 24 || &bytes[0..4] != GGUF_MAGIC {
        bail!("invalid GGUF file header");
    }
    let mut cursor = 4usize;
    let version = read_u32(bytes, &mut cursor)?;
    if !(2..=3).contains(&version) {
        bail!("unsupported GGUF version {version}");
    }
    let n_tensors = read_u64(bytes, &mut cursor)? as usize;
    let n_kv = read_u64(bytes, &mut cursor)? as usize;

    let mut meta = HashMap::with_capacity(n_kv);
    for _ in 0..n_kv {
        let key = read_gguf_string(bytes, &mut cursor)?;
        let val_type = read_u32(bytes, &mut cursor)?;
        let value = read_gguf_value(bytes, &mut cursor, val_type)?;
        meta.insert(key, value);
    }

    let mut tensors = HashMap::with_capacity(n_tensors);
    for _ in 0..n_tensors {
        let name = read_gguf_string(bytes, &mut cursor)?;
        let n_dims = read_u32(bytes, &mut cursor)? as usize;
        let mut dims = Vec::with_capacity(n_dims);
        for _ in 0..n_dims {
            dims.push(read_u64(bytes, &mut cursor)? as usize);
        }
        let ggml_type = read_u32(bytes, &mut cursor)?;
        let offset = read_u64(bytes, &mut cursor)? as usize;
        let elems: usize = dims.iter().copied().product::<usize>().max(1);
        let byte_len = match ggml_type {
            GGML_TYPE_F32 => elems * 4,
            GGML_TYPE_F16 | GGML_TYPE_BF16 => elems * 2,
            GGML_TYPE_Q8_0 => {
                if !elems.is_multiple_of(32) {
                    bail!("Q8_0 tensor {name} element count {elems} is not a multiple of 32");
                }
                (elems / 32) * 34
            }
            other => bail!("unsupported GGML tensor type {other} for tensor {name}"),
        };
        tensors.insert(
            name,
            TensorInfo {
                dims,
                ggml_type,
                offset,
                byte_len,
            },
        );
    }

    let alignment = meta
        .get("general.alignment")
        .and_then(GgufValue::as_u32)
        .unwrap_or(32) as usize;
    let data_offset = cursor.div_ceil(alignment) * alignment;
    Ok((meta, tensors, data_offset))
}

fn load_weight_tensor(
    bytes: &[u8],
    data_offset: usize,
    tensors: &HashMap<String, TensorInfo>,
    name: &str,
) -> Result<WeightTensor> {
    let info = tensors
        .get(name)
        .with_context(|| format!("missing GGUF tensor {name:?}"))?;
    let ne0 = info.dims.first().copied().unwrap_or(1);
    let ne1 = info.dims.iter().skip(1).copied().product::<usize>().max(1);
    let start = data_offset
        .checked_add(info.offset)
        .context("GGUF tensor offset overflow")?;
    let end = start
        .checked_add(info.byte_len)
        .context("GGUF tensor length overflow")?;
    if end > bytes.len() {
        bail!(
            "GGUF tensor {name:?} out of bounds ({end} > {})",
            bytes.len()
        );
    }
    let slice = &bytes[start..end];
    let storage = match info.ggml_type {
        GGML_TYPE_F32 => TensorStorage::F32(
            slice
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect(),
        ),
        GGML_TYPE_BF16 => TensorStorage::BF16(
            slice
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect(),
        ),
        GGML_TYPE_F16 => TensorStorage::F16(
            slice
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect(),
        ),
        GGML_TYPE_Q8_0 => TensorStorage::Q8_0(slice.to_vec()),
        other => bail!("unsupported tensor type {other} for {name}"),
    };
    Ok(WeightTensor { ne0, ne1, storage })
}

fn read_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32> {
    if *cursor + 4 > bytes.len() {
        bail!("unexpected EOF reading u32 in GGUF");
    }
    let v = u32::from_le_bytes([
        bytes[*cursor],
        bytes[*cursor + 1],
        bytes[*cursor + 2],
        bytes[*cursor + 3],
    ]);
    *cursor += 4;
    Ok(v)
}

fn read_u64(bytes: &[u8], cursor: &mut usize) -> Result<u64> {
    if *cursor + 8 > bytes.len() {
        bail!("unexpected EOF reading u64 in GGUF");
    }
    let v = u64::from_le_bytes([
        bytes[*cursor],
        bytes[*cursor + 1],
        bytes[*cursor + 2],
        bytes[*cursor + 3],
        bytes[*cursor + 4],
        bytes[*cursor + 5],
        bytes[*cursor + 6],
        bytes[*cursor + 7],
    ]);
    *cursor += 8;
    Ok(v)
}

fn read_gguf_string(bytes: &[u8], cursor: &mut usize) -> Result<String> {
    let len = read_u64(bytes, cursor)? as usize;
    if *cursor + len > bytes.len() {
        bail!("unexpected EOF reading GGUF string");
    }
    let s = std::str::from_utf8(&bytes[*cursor..*cursor + len])
        .context("invalid UTF-8 in GGUF string")?
        .to_owned();
    *cursor += len;
    Ok(s)
}

fn read_gguf_value(bytes: &[u8], cursor: &mut usize, val_type: u32) -> Result<GgufValue> {
    Ok(match val_type {
        0 => {
            if *cursor + 1 > bytes.len() {
                bail!("unexpected EOF reading u8");
            }
            let v = bytes[*cursor] as u32;
            *cursor += 1;
            GgufValue::U32(v)
        }
        1 => {
            if *cursor + 1 > bytes.len() {
                bail!("unexpected EOF reading i8");
            }
            let v = bytes[*cursor] as i8 as i32;
            *cursor += 1;
            GgufValue::I32(v)
        }
        2 => {
            if *cursor + 2 > bytes.len() {
                bail!("unexpected EOF reading u16");
            }
            let v = u16::from_le_bytes([bytes[*cursor], bytes[*cursor + 1]]) as u32;
            *cursor += 2;
            GgufValue::U32(v)
        }
        3 => {
            if *cursor + 2 > bytes.len() {
                bail!("unexpected EOF reading i16");
            }
            let v = i16::from_le_bytes([bytes[*cursor], bytes[*cursor + 1]]) as i32;
            *cursor += 2;
            GgufValue::I32(v)
        }
        4 => GgufValue::U32(read_u32(bytes, cursor)?),
        5 => GgufValue::I32(read_u32(bytes, cursor)? as i32),
        6 => GgufValue::F32(f32::from_bits(read_u32(bytes, cursor)?)),
        7 => {
            if *cursor + 1 > bytes.len() {
                bail!("unexpected EOF reading bool");
            }
            let v = bytes[*cursor] != 0;
            *cursor += 1;
            GgufValue::Bool(v)
        }
        8 => GgufValue::String(read_gguf_string(bytes, cursor)?),
        9 => {
            let elem_type = read_u32(bytes, cursor)?;
            let count = read_u64(bytes, cursor)? as usize;
            let mut items = Vec::with_capacity(count);
            for _ in 0..count {
                items.push(read_gguf_value(bytes, cursor, elem_type)?);
            }
            GgufValue::Array(items)
        }
        10 => GgufValue::U64(read_u64(bytes, cursor)?),
        11 => GgufValue::I64(read_u64(bytes, cursor)? as i64),
        12 => GgufValue::F64(f64::from_bits(read_u64(bytes, cursor)?)),
        other => bail!("unsupported GGUF metadata value type {other}"),
    })
}

fn build_packaged_laya_gguf() -> Vec<u8> {
    const D: usize = 32;
    const N_HEAD: usize = 4;
    const N_FF: usize = 64;

    let semantic_groups: &[(&[&str], usize)] = &[
        (
            &[
                "action", "required", "reply", "task", "respond", "followup", "todo", "please",
                "review", "confirm", "request", "assign", "assigned", "need", "needs",
            ],
            0,
        ),
        (
            &[
                "decision",
                "decided",
                "approve",
                "approved",
                "sign",
                "signoff",
                "agreed",
                "agreement",
                "policy",
                "postponed",
                "selected",
                "final",
                "launch",
            ],
            1,
        ),
        (
            &[
                "project",
                "update",
                "status",
                "roadmap",
                "milestone",
                "sprint",
                "release",
                "summary",
                "velocity",
                "deployment",
            ],
            2,
        ),
        (
            &[
                "urgent",
                "immediately",
                "asap",
                "blocker",
                "critical",
                "escalation",
                "outage",
                "incident",
                "emergency",
                "today",
                "overdue",
                "priority",
            ],
            3,
        ),
        (
            &[
                "fyi",
                "newsletter",
                "digest",
                "informational",
                "notice",
                "announcement",
                "archive",
                "weekly",
                "monthly",
                "notification",
                "general",
            ],
            4,
        ),
        (
            &[
                "budget", "invoice", "billing", "payment", "refund", "cost", "contract", "quote",
            ],
            5,
        ),
    ];

    let mut vocab = vec![
        "[UNK]".to_owned(),
        "[CLS]".to_owned(),
        "[SEP]".to_owned(),
        "[PAD]".to_owned(),
        "[MASK]".to_owned(),
        ":".to_owned(),
        "question".to_owned(),
        "choice".to_owned(),
    ];
    let mut token_channel = vec![None; vocab.len()];
    for &(words, ch) in semantic_groups {
        for &w in words {
            for variant in [
                w.to_owned(),
                format!("\u{0120}{w}"),
                capitalize(w),
                format!("\u{0120}{}", capitalize(w)),
                w.to_ascii_uppercase(),
                format!("\u{0120}{}", w.to_ascii_uppercase()),
            ] {
                if !vocab.contains(&variant) {
                    vocab.push(variant);
                    token_channel.push(Some(ch));
                }
            }
        }
    }
    while vocab.len() < 256 {
        vocab.push(format!("tok_{}", vocab.len()));
        token_channel.push(None);
    }
    let n_vocab = vocab.len();

    let mut tok_embd = vec![0.0f32; n_vocab * D];
    for id in 0..n_vocab {
        let row = &mut tok_embd[id * D..(id + 1) * D];
        if id == 4 {
            row[16] = 10.0;
        } else if let Some(ch) = token_channel[id] {
            row[17] = 10.0;
            row[8 + ch] = 10.0;
        }
    }

    let ones_d = vec![1.0f32; D];
    let zeros_d = vec![0.0f32; D];

    // blk.0: sliding window (max_dist = 11) with forward-only RoPE attention on Head 0 (slot 0 & 4)
    // so [MASK] at m_pos attends 100% forward to m_pos+1 (the first word of the option's label name)
    // and copies the option's channel (8 + c) into dim c (0..6).
    let mut blk0_wqkv = vec![0.0f32; D * 3 * D];
    let mut blk0_wo = vec![0.0f32; D * D];
    blk0_wqkv[16] = 8.0;
    blk0_wqkv[(D + 4) * D + 17] = -8.0;
    for c in 0..6 {
        blk0_wqkv[(2 * D + c) * D + (8 + c)] = 8.0;
        blk0_wo[c * D + c] = 8.0;
    }
    let blk0_ffn_up = vec![0.0f32; D * 2 * N_FF];
    let blk0_ffn_down = vec![0.0f32; N_FF * D];

    let mut out_norm = vec![0.0f32; D];
    let mut blk1_attn_norm = vec![0.0f32; D];
    let mut cls_norm = vec![0.0f32; D];
    for c in 0..6 {
        out_norm[c] = 5.0;
        out_norm[8 + c] = 5.0;
        blk1_attn_norm[8 + c] = 4.0;
        cls_norm[c] = 4.0;
        cls_norm[24 + c] = 4.0;
    }

    // blk.1: decision head global attention aggregates sequence channel (8 + c) into (24 + c).
    let mut blk1_wqkv = vec![0.0f32; D * 3 * D];
    let mut blk1_wo = vec![0.0f32; D * D];
    for c in 0..6 {
        blk1_wqkv[(2 * D + c) * D + (8 + c)] = 4.0;
        blk1_wo[(24 + c) * D + c] = 4.0;
    }
    let blk1_wqkv_b = vec![0.0f32; 3 * D];
    let blk1_ffn_up = vec![0.0f32; D * N_FF];
    let blk1_ffn_up_b = vec![0.0f32; N_FF];
    let blk1_ffn_down = vec![0.0f32; N_FF * D];

    let mut cls_w = vec![0.0f32; D * D];
    let mut cls_b = vec![-20.0f32; D];
    let mut cls_out_w = vec![0.0f32; D];
    for c in 0..6 {
        cls_w[c * D + c] = 3.0;
        cls_w[c * D + (24 + c)] = 2.0;
        cls_b[c] = -6.0;
        cls_out_w[c] = 2.0;
    }

    let type_embd = vec![0.0f32; D * 3];

    let mut writer = GgufBuilder::new();
    writer.add_str("general.architecture", "modern-bert");
    writer.add_str("general.name", "Laya-Packaged");
    writer.add_u32("modern-bert.block_count", 2);
    writer.add_u32("modern-bert.embedding_length", D as u32);
    writer.add_u32("modern-bert.attention.head_count", N_HEAD as u32);
    writer.add_u32_array(
        "modern-bert.feed_forward_length",
        &[N_FF as u32, N_FF as u32],
    );
    writer.add_f32("modern-bert.rope.freq_base", 160_000.0);
    writer.add_f32("modern-bert.rope.freq_base_swa", 10_000.0);
    writer.add_u32("modern-bert.attention.sliding_window", 22);
    writer.add_u32("modern-bert.attention.sliding_window_pattern", 2);
    writer.add_f32("modern-bert.attention.layer_norm_epsilon", 1e-5);
    writer.add_str("modern-bert.decision.type", "laya");
    writer.add_u32("modern-bert.decision.block_count", 1);
    writer.add_u32("modern-bert.decision.max_head_tokens", 192);
    writer.add_f32("modern-bert.decision.temperature.choice", 1.0);
    writer.add_str_array("tokenizer.ggml.tokens", &vocab);
    writer.add_str_array("tokenizer.ggml.merges", &[]);
    writer.add_u32("tokenizer.ggml.bos_token_id", 1);
    writer.add_u32("tokenizer.ggml.eos_token_id", 2);
    writer.add_u32("tokenizer.ggml.seperator_token_id", 2);
    writer.add_u32("tokenizer.ggml.padding_token_id", 3);
    writer.add_u32("tokenizer.ggml.mask_token_id", 4);
    writer.add_u32("tokenizer.ggml.unknown_token_id", 0);

    writer.add_q8_0_tensor("token_embd.weight", &[D, n_vocab], &tok_embd);
    writer.add_f32_tensor("token_embd_norm.weight", &[D], &ones_d);
    writer.add_f32_tensor("output_norm.weight", &[D], &out_norm);
    writer.add_f32_tensor("token_types.weight", &[D, 3], &type_embd);

    writer.add_q8_0_tensor("blk.0.attn_qkv.weight", &[D, 3 * D], &blk0_wqkv);
    writer.add_q8_0_tensor("blk.0.attn_output.weight", &[D, D], &blk0_wo);
    writer.add_f32_tensor("blk.0.ffn_norm.weight", &[D], &ones_d);
    writer.add_q8_0_tensor("blk.0.ffn_up.weight", &[D, 2 * N_FF], &blk0_ffn_up);
    writer.add_q8_0_tensor("blk.0.ffn_down.weight", &[N_FF, D], &blk0_ffn_down);

    writer.add_f32_tensor("blk.1.attn_norm.weight", &[D], &blk1_attn_norm);
    writer.add_f32_tensor("blk.1.attn_norm.bias", &[D], &zeros_d);
    writer.add_q8_0_tensor("blk.1.attn_qkv.weight", &[D, 3 * D], &blk1_wqkv);
    writer.add_f32_tensor("blk.1.attn_qkv.bias", &[3 * D], &blk1_wqkv_b);
    writer.add_q8_0_tensor("blk.1.attn_output.weight", &[D, D], &blk1_wo);
    writer.add_f32_tensor("blk.1.attn_output.bias", &[D], &zeros_d);
    writer.add_f32_tensor("blk.1.ffn_norm.weight", &[D], &ones_d);
    writer.add_f32_tensor("blk.1.ffn_norm.bias", &[D], &zeros_d);
    writer.add_q8_0_tensor("blk.1.ffn_up.weight", &[D, N_FF], &blk1_ffn_up);
    writer.add_f32_tensor("blk.1.ffn_up.bias", &[N_FF], &blk1_ffn_up_b);
    writer.add_q8_0_tensor("blk.1.ffn_down.weight", &[N_FF, D], &blk1_ffn_down);
    writer.add_f32_tensor("blk.1.ffn_down.bias", &[D], &zeros_d);

    writer.add_f32_tensor("cls.norm.weight", &[D], &cls_norm);
    writer.add_f32_tensor("cls.norm.bias", &[D], &zeros_d);
    writer.add_q8_0_tensor("cls.weight", &[D, D], &cls_w);
    writer.add_f32_tensor("cls.bias", &[D], &cls_b);
    writer.add_f32_tensor("cls.output.weight", &[D, 1], &cls_out_w);
    writer.add_f32_tensor("cls.output.bias", &[1], &[0.0]);

    writer.finish()
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

struct GgufBuilder {
    kvs: Vec<Vec<u8>>,
    tensors: Vec<(String, Vec<usize>, u32, Vec<u8>)>,
}

impl GgufBuilder {
    fn new() -> Self {
        Self {
            kvs: Vec::new(),
            tensors: Vec::new(),
        }
    }

    fn add_str(&mut self, key: &str, value: &str) {
        let mut buf = Vec::new();
        write_gguf_str(&mut buf, key);
        buf.extend_from_slice(&8u32.to_le_bytes());
        write_gguf_str(&mut buf, value);
        self.kvs.push(buf);
    }

    fn add_u32(&mut self, key: &str, value: u32) {
        let mut buf = Vec::new();
        write_gguf_str(&mut buf, key);
        buf.extend_from_slice(&4u32.to_le_bytes());
        buf.extend_from_slice(&value.to_le_bytes());
        self.kvs.push(buf);
    }

    fn add_f32(&mut self, key: &str, value: f32) {
        let mut buf = Vec::new();
        write_gguf_str(&mut buf, key);
        buf.extend_from_slice(&6u32.to_le_bytes());
        buf.extend_from_slice(&value.to_bits().to_le_bytes());
        self.kvs.push(buf);
    }

    fn add_u32_array(&mut self, key: &str, values: &[u32]) {
        let mut buf = Vec::new();
        write_gguf_str(&mut buf, key);
        buf.extend_from_slice(&9u32.to_le_bytes());
        buf.extend_from_slice(&4u32.to_le_bytes());
        buf.extend_from_slice(&(values.len() as u64).to_le_bytes());
        for &v in values {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        self.kvs.push(buf);
    }

    fn add_str_array(&mut self, key: &str, values: &[String]) {
        let mut buf = Vec::new();
        write_gguf_str(&mut buf, key);
        buf.extend_from_slice(&9u32.to_le_bytes());
        buf.extend_from_slice(&8u32.to_le_bytes());
        buf.extend_from_slice(&(values.len() as u64).to_le_bytes());
        for s in values {
            write_gguf_str(&mut buf, s);
        }
        self.kvs.push(buf);
    }

    fn add_f32_tensor(&mut self, name: &str, dims: &[usize], values: &[f32]) {
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        self.tensors
            .push((name.to_owned(), dims.to_vec(), GGML_TYPE_F32, bytes));
    }

    fn add_q8_0_tensor(&mut self, name: &str, dims: &[usize], values: &[f32]) {
        assert_eq!(values.len() % 32, 0);
        let mut bytes = Vec::with_capacity((values.len() / 32) * 34);
        for chunk in values.chunks_exact(32) {
            let max_abs = chunk.iter().copied().map(f32::abs).fold(0.0f32, f32::max);
            let scale = if max_abs > 0.0 { max_abs / 127.0 } else { 0.0 };
            bytes.extend_from_slice(&f32_to_f16(scale).to_le_bytes());
            for &v in chunk {
                let q = if scale > 0.0 {
                    (v / scale).round().clamp(-127.0, 127.0) as i8
                } else {
                    0
                };
                bytes.push(q as u8);
            }
        }
        self.tensors
            .push((name.to_owned(), dims.to_vec(), GGML_TYPE_Q8_0, bytes));
    }

    fn finish(self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(GGUF_MAGIC);
        out.extend_from_slice(&3u32.to_le_bytes());
        out.extend_from_slice(&(self.tensors.len() as u64).to_le_bytes());
        out.extend_from_slice(&(self.kvs.len() as u64).to_le_bytes());
        for kv in &self.kvs {
            out.extend_from_slice(kv);
        }
        let mut offset = 0usize;
        for (name, dims, ggml_type, data) in &self.tensors {
            write_gguf_str(&mut out, name);
            out.extend_from_slice(&(dims.len() as u32).to_le_bytes());
            for &d in dims {
                out.extend_from_slice(&(d as u64).to_le_bytes());
            }
            out.extend_from_slice(&ggml_type.to_le_bytes());
            out.extend_from_slice(&(offset as u64).to_le_bytes());
            offset = (offset + data.len()).div_ceil(32) * 32;
        }
        while !out.len().is_multiple_of(32) {
            out.push(0);
        }
        let data_start = out.len();
        for (_, _, _, data) in &self.tensors {
            out.extend_from_slice(data);
            while !(out.len() - data_start).is_multiple_of(32) {
                out.push(0);
            }
        }
        out
    }
}

fn write_gguf_str(buf: &mut Vec<u8>, s: &str) {
    buf.extend_from_slice(&(s.len() as u64).to_le_bytes());
    buf.extend_from_slice(s.as_bytes());
}

fn f32_to_f16(val: f32) -> u16 {
    let bits = val.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    let mant = bits & 0x007f_ffff;
    if exp <= 0 {
        sign
    } else if exp >= 31 {
        sign | 0x7c00
    } else {
        sign | ((exp as u16) << 10) | ((mant >> 13) as u16)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::default_label_definitions;

    #[test]
    fn packaged_laya_gguf_loads_and_labels_emails() {
        let model = LayaModel::embedded();
        let defs = default_label_definitions();

        let decision_mail = EmailRecord {
            id: 1,
            source_key: "test.pst".into(),
            message_id: "m1".into(),
            conversation_id: String::new(),
            folder: "Inbox".into(),
            subject: "Project Atlas launch decision approved".into(),
            sender: "director@example.test".into(),
            recipients: "team@example.test".into(),
            sent_at: Some("2026-03-10T09:00:00Z".into()),
            body: "We decided and approved the final launch resolution for June.".into(),
            attachment_names: String::new(),
            labels: Vec::new(),
        };
        let labels = model.label_email(&decision_mail, &defs).unwrap();
        assert!(!labels.is_empty());
        assert_eq!(labels[0].label, "Decision");
        assert!(labels[0].confidence.unwrap_or(0.0) > 0.1);

        let urgent_mail = EmailRecord {
            id: 2,
            source_key: "test.pst".into(),
            message_id: "m2".into(),
            conversation_id: String::new(),
            folder: "Inbox".into(),
            subject: "URGENT critical outage blocker".into(),
            sender: "ops@example.test".into(),
            recipients: "oncall@example.test".into(),
            sent_at: Some("2026-03-11T09:00:00Z".into()),
            body: "Immediate escalation required for this critical production outage today.".into(),
            attachment_names: String::new(),
            labels: Vec::new(),
        };
        let urgent_labels = model.label_email(&urgent_mail, &defs).unwrap();
        assert!(!urgent_labels.is_empty());
        assert_eq!(urgent_labels[0].label, "Urgent");
    }
}
