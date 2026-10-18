use super::gguf::{GGML_TYPE_F32, GGML_TYPE_Q8_0, GGUF_MAGIC, f32_to_f16};

pub(super) fn build_packaged_laya_gguf() -> Vec<u8> {
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
