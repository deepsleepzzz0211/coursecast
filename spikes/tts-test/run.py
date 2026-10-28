"""MiMo TTS 实测：音色复刻 + 预置音色，测量延迟与实时率。

用法：
    MIMO_API_KEY=xxx .venv/Scripts/python.exe spikes/tts-test/run.py [--pages N]
"""

import base64
import json
import os
import sys
import time
import urllib.request
import urllib.error
from pathlib import Path

ENDPOINT = "https://api.xiaomimimo.com/v1/chat/completions"
DATA = Path(__file__).resolve().parents[2] / "data" / "tts-test"

# 真实课件风格的讲稿（取自《第五章 药品包装机械》）
SENTENCES = [
    "各位同学好，这一章我们讲药品包装机械。包装是药品生产过程中的重要环节，它直接关系到药品的质量和安全。",
    "我们先看包装机械的分类。按功能划分，可以分为充填机械、灌装机械、封口机械、贴标机械和装盒机械五大类。",
    "充填机械的作用是把定量的物料装入包装容器。常见的充填方式有称重式、容积式和计数式三种。",
    "灌装机械主要用于液体药品。它通过计量装置控制每次灌装的体积，精度通常在正负百分之一以内。",
    "封口是包装的最后一道工序。封口质量直接决定了药品在储存和运输过程中是否会发生泄漏或污染。",
]


def post(payload: dict, timeout: int = 180):
    body = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(
        ENDPOINT,
        data=body,
        headers={
            "api-key": os.environ["MIMO_API_KEY"],
            "Content-Type": "application/json",
        },
        method="POST",
    )
    t0 = time.perf_counter()
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            raw = r.read()
    except urllib.error.HTTPError as e:
        detail = e.read().decode("utf-8", errors="ignore")[:500]
        raise RuntimeError(f"HTTP {e.code}: {detail}") from e
    dt = time.perf_counter() - t0
    return json.loads(raw), dt


def synth(text: str, model: str, voice: str, fmt: str = "wav"):
    payload = {
        "model": model,
        "messages": [
            {"role": "user", "content": "语速适中，讲解清晰，自然停顿，像在课堂上讲课"},
            {"role": "assistant", "content": text},
        ],
        "audio": {"format": fmt, "voice": voice},
    }
    resp, dt = post(payload)
    msg = resp["choices"][0]["message"]
    audio_b64 = msg["audio"]["data"]
    audio = base64.b64decode(audio_b64)
    usage = resp.get("usage", {})
    return audio, dt, usage


def wav_duration(data: bytes) -> float:
    """从 wav 头读时长（pcm）。"""
    import struct
    if len(data) < 44 or data[:4] != b"RIFF":
        return 0.0
    pos = 12
    byte_rate = 0
    data_size = 0
    while pos + 8 <= len(data):
        cid = data[pos:pos + 4]
        sz = struct.unpack("<I", data[pos + 4:pos + 8])[0]
        if cid == b"fmt ":
            byte_rate = struct.unpack("<I", data[pos + 16:pos + 20])[0]
        elif cid == b"data":
            data_size = sz
            break
        pos += 8 + sz + (sz & 1)
    return data_size / byte_rate if byte_rate else 0.0


def main():
    n = 1
    if "--pages" in sys.argv:
        n = int(sys.argv[sys.argv.index("--pages") + 1])

    ref = DATA / "voice_ref.wav"
    b64 = base64.b64encode(ref.read_bytes()).decode()
    voice_uri = f"data:audio/wav;base64,{b64}"

    print(f"参考音频 : {ref.name}  {ref.stat().st_size/1024:.0f} KB  5.97 s")
    print(f"base64   : {len(voice_uri)/1024/1024:.2f} MB （上限 10 MB）")
    print("=" * 68)

    results = []
    for i, text in enumerate(SENTENCES[:n], 1):
        try:
            audio, dt, usage = synth(text, "mimo-v2.5-tts-voiceclone", voice_uri)
            dur = wav_duration(audio)
            out = DATA / f"clone_{i:02d}.wav"
            out.write_bytes(audio)
            rtf = dt / dur if dur else 0
            results.append((i, len(text), dt, dur, rtf, len(audio)))
            print(f"[克隆 {i}] {len(text):3d} 字 | 请求 {dt:6.2f}s | "
                  f"音频 {dur:5.2f}s | RTF {rtf:4.2f} | {len(audio)/1024:.0f} KB "
                  f"| tokens {usage.get('total_tokens','?')}")
        except Exception as e:
            print(f"[克隆 {i}] 失败: {e}")

    if results:
        total_req = sum(r[2] for r in results)
        total_audio = sum(r[3] for r in results)
        print("-" * 68)
        print(f"合计：{len(results)} 段 | 请求耗时 {total_req:.2f}s | "
              f"音频时长 {total_audio:.2f}s | 整体 RTF {total_req/total_audio:.2f}")
        print(f"外推 29 页（串行，平均每页 20s 音频）："
              f"约 {total_req/len(results)*29/60:.1f} 分钟")

    print("=" * 68)
    print("对照组：预置音色 mimo-v2.5-tts / 冰糖")
    try:
        audio, dt, usage = synth(SENTENCES[0], "mimo-v2.5-tts", "冰糖")
        dur = wav_duration(audio)
        (DATA / "preset_01.wav").write_bytes(audio)
        print(f"  请求 {dt:6.2f}s | 音频 {dur:5.2f}s | RTF {dt/dur if dur else 0:4.2f}")
    except Exception as e:
        print(f"  失败: {e}")


if __name__ == "__main__":
    sys.exit(main())
