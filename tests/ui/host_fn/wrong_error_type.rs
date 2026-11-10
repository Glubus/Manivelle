use manivelle::host_fn;

#[derive(Debug)]
struct MathError;

#[host_fn(name = "math.one")]
fn one() -> Result<f64, MathError> {
    Ok(1.0)
}

fn main() {}
