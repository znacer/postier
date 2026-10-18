pub(super) fn layer_norm(
    input: &[f32],
    weight: &[f32],
    bias: Option<&[f32]>,
    eps: f32,
    out: &mut [f32],
) {
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

pub(super) fn apply_rope_neox(
    vec: &mut [f32],
    n_head: usize,
    head_dim: usize,
    pos: usize,
    freq_base: f32,
) {
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

pub(super) fn multihead_attention(
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

pub(super) fn gelu_erf(x: f32) -> f32 {
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
