mod gguf;
mod ops;
mod packaged;
mod tokenizer;

use std::{
    collections::HashMap,
    fs::File,
    io::{BufReader, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

use anyhow::{Context, Result, bail};

use crate::model::{AssignedLabel, EmailRecord, LabelDefinition};

use self::{
    gguf::{GgufValue, WeightTensor, load_weight_tensor, parse_gguf},
    ops::{apply_rope_neox, gelu_erf, layer_norm, multihead_attention},
    packaged::build_packaged_laya_gguf,
    tokenizer::BpeTokenizer,
};

#[allow(dead_code)]
pub const HF_LAYA_REPO: &str = "https://huggingface.co/ggml-org/Laya-GGUF";
pub const HF_LAYA_Q8_URL: &str =
    "https://huggingface.co/ggml-org/Laya-GGUF/resolve/main/Laya-Q8_0.gguf";
pub const DEFAULT_LAYA_FILENAME: &str = "Laya-Q8_0.gguf";

type CachedLayaModel = (PathBuf, Arc<LayaModel>);

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
