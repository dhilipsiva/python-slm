use half::bf16;

#[derive(Clone, Debug, PartialEq)]
pub struct OracleResult {
    pub y: Vec<f64>,
    pub loss: f64,
    pub grad_a: Vec<f64>,
    pub grad_b: Vec<f64>,
}

pub fn evaluate_oracle(a: &[bf16], b: &[bf16], m: usize, k: usize, n: usize) -> OracleResult {
    assert_eq!(a.len(), m * k, "A shape mismatch");
    assert_eq!(b.len(), k * n, "B shape mismatch");
    let a = a
        .iter()
        .map(|value| f64::from(value.to_f32()))
        .collect::<Vec<_>>();
    let b = b
        .iter()
        .map(|value| f64::from(value.to_f32()))
        .collect::<Vec<_>>();
    let mut y = vec![0.0_f64; m * n];
    for row in 0..m {
        for column in 0..n {
            let mut sum = 0.0_f64;
            for inner in 0..k {
                sum += a[row * k + inner] * b[inner * n + column];
            }
            y[row * n + column] = sum;
        }
    }
    let output_elements = (m * n) as f64;
    let loss = y.iter().map(|value| value * value).sum::<f64>() / output_elements;
    let loss_scale = 2.0 / output_elements;

    let mut grad_a = vec![0.0_f64; m * k];
    for row in 0..m {
        for inner in 0..k {
            let mut sum = 0.0_f64;
            for column in 0..n {
                sum += y[row * n + column] * b[inner * n + column];
            }
            grad_a[row * k + inner] = loss_scale * sum;
        }
    }

    let mut grad_b = vec![0.0_f64; k * n];
    for inner in 0..k {
        for column in 0..n {
            let mut sum = 0.0_f64;
            for row in 0..m {
                sum += a[row * k + inner] * y[row * n + column];
            }
            grad_b[inner * n + column] = loss_scale * sum;
        }
    }

    OracleResult {
        y,
        loss,
        grad_a,
        grad_b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(value: f32) -> bf16 {
        bf16::from_f32(value)
    }

    #[test]
    fn oracle_matches_hand_computed_two_by_two_graph() {
        let result = evaluate_oracle(
            &[b(1.0), b(2.0), b(3.0), b(4.0)],
            &[b(5.0), b(6.0), b(7.0), b(8.0)],
            2,
            2,
            2,
        );
        assert_eq!(result.y, vec![19.0, 22.0, 43.0, 50.0]);
        assert_eq!(result.loss, 1_298.5);
        assert_eq!(result.grad_a, vec![113.5, 154.5, 257.5, 350.5]);
        assert_eq!(result.grad_b, vec![74.0, 86.0, 105.0, 122.0]);
    }

    #[test]
    fn zero_graph_has_zero_gradients() {
        let result = evaluate_oracle(&[b(0.0); 6], &[b(0.0); 6], 2, 3, 2);
        assert_eq!(result.loss, 0.0);
        assert!(result.grad_a.iter().all(|&value| value == 0.0));
        assert!(result.grad_b.iter().all(|&value| value == 0.0));
    }
}
