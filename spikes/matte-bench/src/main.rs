use anyhow::{Context, Result};
use ort::session::Session;
use ort::value::Tensor;
use std::time::Instant;

const W: usize = 512;
const H: usize = 288;

fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "../../vendor/rvm_mobilenetv3_fp32.onnx".to_string());
    let iters: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);

    println!("model  : {path}");
    println!("iters  : {iters}");

    let t0 = Instant::now();
    let mut sess = Session::builder()?
        .commit_from_file(&path)
        .with_context(|| format!("load {path}"))?;
    println!("load   : {:.0} ms", t0.elapsed().as_secs_f64() * 1000.0);

    let ins: Vec<String> = sess.inputs().iter().map(|i| i.name().to_string()).collect();
    let outs: Vec<String> = sess.outputs().iter().map(|o| o.name().to_string()).collect();
    println!("inputs : {ins:?}");
    println!("outputs: {outs:?}");

    // 预热：让 ORT 完成算子融合与内存池分配
    for _ in 0..3 {
        run_once(&mut sess)?;
    }

    let t1 = Instant::now();
    for _ in 0..iters {
        run_once(&mut sess)?;
    }
    let secs = t1.elapsed().as_secs_f64();
    let fps = iters as f64 / secs;

    println!("--------------------------------------------------");
    println!("resolution : {W}x{H}");
    println!("total      : {:.3} s / {iters} frames", secs);
    println!("per frame  : {:.1} ms", secs * 1000.0 / iters as f64);
    println!("FPS        : {fps:.2}");
    Ok(())
}

fn run_once(sess: &mut Session) -> Result<()> {
    let src = Tensor::from_array(([1usize, 3, H, W], vec![0.5f32; 3 * H * W]))?;
    let r1 = Tensor::from_array(([1usize, 1, 1, 1], vec![0f32; 1]))?;
    let r2 = Tensor::from_array(([1usize, 1, 1, 1], vec![0f32; 1]))?;
    let r3 = Tensor::from_array(([1usize, 1, 1, 1], vec![0f32; 1]))?;
    let r4 = Tensor::from_array(([1usize, 1, 1, 1], vec![0f32; 1]))?;
    let dsr = Tensor::from_array(([1usize], vec![0.375f32]))?;

    let _out = sess.run(ort::inputs![
        "src" => src,
        "r1i" => r1,
        "r2i" => r2,
        "r3i" => r3,
        "r4i" => r4,
        "downsample_ratio" => dsr,
    ])?;
    Ok(())
}
