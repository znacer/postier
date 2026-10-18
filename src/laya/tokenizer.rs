use std::collections::HashMap;

use anyhow::{Context, Result};

use super::gguf::GgufValue;

#[derive(Clone)]
pub(super) struct BpeTokenizer {
    pub(super) token_to_id: HashMap<String, u32>,
    pub(super) bpe_ranks: HashMap<(String, String), usize>,
    pub(super) byte_encoder: [char; 256],
    pub(super) cls_id: u32,
    pub(super) sep_id: u32,
    pub(super) mask_id: u32,
    pub(super) unk_id: u32,
}

impl BpeTokenizer {
    pub(super) fn from_gguf_meta(meta: &HashMap<String, GgufValue>) -> Result<Self> {
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

    pub(super) fn encode(&self, text: &str) -> Vec<u32> {
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
