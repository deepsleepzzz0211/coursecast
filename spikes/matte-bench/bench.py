"""RVM / MODNet 抠像推理速度实测（ONNX Runtime，CPU）。

用法：
    .venv/Scripts/python.exe spikes/matte-bench/bench.py
"""

import time
import sys
from pathlib import Path

import numpy as np
import onnxruntime as ort

ROOT = Path(__file__).resolve().parents[2]
VENDOR = ROOT / "vendor"


def describe(path: Path):
    sess = ort.InferenceSession(str(path), providers=["CPUExecutionProvider"])
    ins = [(i.name, i.shape, i.type) for i in sess.get_inputs()]
    outs = [(o.name, o.shape, o.type) for o in sess.get_outputs()]
    return sess, ins, outs


def bench_rvm(path: Path, w: int, h: int, iters: int = 30):
    t0 = time.perf_counter()
    sess = ort.InferenceSession(str(path), providers=["CPUExecutionProvider"])
    load_ms = (time.perf_counter() - t0) * 1000

    src = np.random.rand(1, 3, h, w).astype(np.float32)
    rec = [np.zeros((1, 1, 1, 1), dtype=np.float32) for _ in range(4)]
    dsr = np.array([0.375], dtype=np.float32)

    feed = {
        "src": src,
        "r1i": rec[0], "r2i": rec[1], "r3i": rec[2], "r4i": rec[3],
        "downsample_ratio": dsr,
    }
    names = [o.name for o in sess.get_outputs()]

    for _ in range(3):
        sess.run(names, feed)

    t0 = time.perf_counter()
    for _ in range(iters):
        sess.run(names, feed)
    secs = time.perf_counter() - t0
    return load_ms, secs / iters * 1000, iters / secs


def bench_modnet(path: Path, size: int = 512, iters: int = 30):
    t0 = time.perf_counter()
    sess = ort.InferenceSession(str(path), providers=["CPUExecutionProvider"])
    load_ms = (time.perf_counter() - t0) * 1000

    iname = sess.get_inputs()[0].name
    onames = [o.name for o in sess.get_outputs()]
    x = np.random.rand(1, 3, size, size).astype(np.float32)
    feed = {iname: x}

    for _ in range(3):
        sess.run(onames, feed)

    t0 = time.perf_counter()
    for _ in range(iters):
        sess.run(onames, feed)
    secs = time.perf_counter() - t0
    return load_ms, secs / iters * 1000, iters / secs


def main():
    print(f"onnxruntime {ort.__version__}")
    print(f"providers   {ort.get_available_providers()}")
    print("=" * 62)

    targets = [
        ("RVM mobilenetv3 fp32", VENDOR / "rvm_mobilenetv3_fp32.onnx", "rvm", (512, 288)),
        ("RVM mobilenetv3 fp32", VENDOR / "rvm_mobilenetv3_fp32.onnx", "rvm", (1280, 720)),
        ("RVM mobilenetv3 fp32", VENDOR / "rvm_mobilenetv3_fp32.onnx", "rvm", (1920, 1080)),
        ("MODNet fp32",          VENDOR / "modnet_fp32.onnx",            "modnet", 512),
        ("MODNet quantized",     VENDOR / "modnet_quantized.onnx",       "modnet", 512),
    ]

    for label, path, kind, size in targets:
        if not path.exists():
            print(f"{label:24s} 缺失: {path.name}")
            continue
        try:
            if kind == "rvm":
                w, h = size
                load_ms, per_ms, fps = bench_rvm(path, w, h)
                dims = f"{w}x{h}"
            else:
                load_ms, per_ms, fps = bench_modnet(path, size)
                dims = f"{size}x{size}"
            print(f"{label:24s} {dims:10s} 加载 {load_ms:6.0f} ms | "
                  f"单帧 {per_ms:7.1f} ms | {fps:6.2f} FPS")
        except Exception as e:
            print(f"{label:24s} 失败: {type(e).__name__}: {e}")


if __name__ == "__main__":
    sys.exit(main())
