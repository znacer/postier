use std::collections::HashMap;

use anyhow::{Context, Result, bail};

pub(super) const GGUF_MAGIC: &[u8; 4] = b"GGUF";
pub(super) const GGML_TYPE_F32: u32 = 0;
pub(super) const GGML_TYPE_F16: u32 = 1;
pub(super) const GGML_TYPE_Q8_0: u32 = 8;
pub(super) const GGML_TYPE_BF16: u32 = 30;

pub(super) type GgufParsed = (
    HashMap<String, GgufValue>,
    HashMap<String, TensorInfo>,
    usize,
);

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(super) enum GgufValue {
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
    pub(super) fn as_u32(&self) -> Option<u32> {
        match self {
            Self::U32(v) => Some(*v),
            Self::I32(v) => (*v >= 0).then_some(*v as u32),
            Self::U64(v) => Some(*v as u32),
            Self::I64(v) => (*v >= 0).then_some(*v as u32),
            _ => None,
        }
    }

    pub(super) fn as_f32(&self) -> Option<f32> {
        match self {
            Self::F32(v) => Some(*v),
            Self::F64(v) => Some(*v as f32),
            Self::U32(v) => Some(*v as f32),
            Self::I32(v) => Some(*v as f32),
            _ => None,
        }
    }

    pub(super) fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub(super) fn as_u32_vec(&self) -> Option<Vec<u32>> {
        match self {
            Self::Array(items) => items.iter().map(Self::as_u32).collect(),
            _ => None,
        }
    }

    pub(super) fn as_string_vec(&self) -> Option<Vec<String>> {
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
pub(super) struct TensorInfo {
    pub(super) dims: Vec<usize>,
    pub(super) ggml_type: u32,
    pub(super) offset: usize,
    pub(super) byte_len: usize,
}

#[derive(Clone)]
pub(super) enum TensorStorage {
    F32(Vec<f32>),
    BF16(Vec<u16>),
    F16(Vec<u16>),
    Q8_0(Vec<u8>),
}

#[derive(Clone)]
pub(super) struct WeightTensor {
    pub(super) ne0: usize,
    pub(super) ne1: usize,
    pub(super) storage: TensorStorage,
}

impl WeightTensor {
    pub(super) fn get_row(&self, row: usize, out: &mut [f32]) {
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

    pub(super) fn dot_row(&self, row: usize, input: &[f32]) -> f32 {
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

    pub(super) fn linear(&self, input: &[f32], bias: Option<&[f32]>, out: &mut [f32]) {
        debug_assert_eq!(input.len(), self.ne0);
        debug_assert_eq!(out.len(), self.ne1);
        for (row, dst) in out.iter_mut().enumerate() {
            let b = bias.map_or(0.0, |b| b[row]);
            *dst = self.dot_row(row, input) + b;
        }
    }
}

pub(super) fn bf16_to_f32(bits: u16) -> f32 {
    f32::from_bits((bits as u32) << 16)
}

pub(super) fn f16_to_f32(bits: u16) -> f32 {
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

pub(super) fn f32_to_f16(val: f32) -> u16 {
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

pub(super) fn parse_gguf(bytes: &[u8]) -> Result<GgufParsed> {
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

pub(super) fn load_weight_tensor(
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
