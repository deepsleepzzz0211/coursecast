//! 故障注入②的 ffmpeg 垫片（票 16）：`CC_FFMPEG` 指向本程序，其余调用原样转发。
//!
//! - 默认（`CC_SHIM_FAIL_RENDER` 未设）：把参数转发给 `CC_SHIM_REAL_FFMPEG`，用于
//!   「同一垫片路径下单阶段重试成功」。
//! - `CC_SHIM_FAIL_RENDER=1` 时，对渲染调用（唯一带 `-filter_complex` 的那次）先写
//!   半截输出再退出 1——模拟 ffmpeg 渲染中途失败、磁盘上留下脏产物。
//!
//! 编译：rustc -O scripts/acceptance/ffmpeg-shim.rs -o <tmp>/ffmpeg-shim.exe

use std::io::Write;
use std::process::Command;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let fail_render = std::env::var("CC_SHIM_FAIL_RENDER").as_deref() == Ok("1");
    let is_render = args.iter().any(|a| a == "-filter_complex");

    if fail_render && is_render {
        if let Some(out) = args.iter().rev().find(|a| a.ends_with(".mp4")) {
            if let Ok(mut f) = std::fs::File::create(out) {
                let _ = f.write_all(b"partial render left by the acceptance shim");
            }
        }
        eprintln!("acceptance injection: ffmpeg failed mid-render");
        std::process::exit(1);
    }

    let real = std::env::var("CC_SHIM_REAL_FFMPEG")
        .expect("CC_SHIM_REAL_FFMPEG must point at the real ffmpeg binary");
    let status = Command::new(real)
        .args(&args)
        .status()
        .expect("failed to spawn the real ffmpeg");
    std::process::exit(status.code().unwrap_or(1));
}
