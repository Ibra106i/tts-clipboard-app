#!/usr/bin/env python3
"""
Local Inflect v2 wrapper for the Tauri clipboard reader.

The app drives this script as a short-lived subprocess per chunk (or per
short synthesis request). Contract:

    python inflect_speak.py \
        --model-dir <cached model path> \
        --model {micro,nano} \
        --text "<chunk text>" \
        --speed 1.0 \
        --variation 0.667 \
        --seed 7 \
        --output chunk.wav

Exit 0 with a written WAV on success. Exit non-zero with a printable reason on
failure. No other stdout/stderr contract is promised to the caller; diagnostics
go to stderr only.
"""

import argparse
import sys
from pathlib import Path


def main() -> int:
    p = argparse.ArgumentParser(
        description="Synthesize one chunk of text with a local Inflect v2 model."
    )
    p.add_argument("--model-dir", required=True, type=Path)
    p.add_argument(
        "--model",
        required=True,
        choices=("micro", "nano"),
        help="Which Inflect v2 checkpoint is cached under --model-dir.",
    )
    p.add_argument("--text", required=True, type=str)
    p.add_argument(
        "--speed",
        type=float,
        default=1.0,
        help="Speaking speed multiplier (Inflect 'speed').",
    )
    p.add_argument(
        "--variation",
        type=float,
        default=0.667,
        help="Delivery variation (Inflect 'variation').",
    )
    p.add_argument(
        "--pitch",
        type=float,
        default=None,
        help="Restrained pitch adjustment when provided.",
    )
    p.add_argument("--seed", type=int, default=None)
    p.add_argument(
        "--output",
        required=True,
        type=Path,
        help="Where to write the synthesized WAV.",
    )
    args = p.parse_args()

    if not args.model_dir.is_dir():
        print(
            f"Inflect model directory not found: {args.model_dir}",
            file=sys.stderr,
        )
        return 2

    try:
        from inference import InflectTTS
    except ImportError as exc:
        print(
            f"Cannot import InflectTTS from {args.model_dir!r}: {exc}",
            file=sys.stderr,
        )
        return 3

    try:
        tts = InflectTTS(str(args.model_dir), device="cpu")
    except Exception as exc:
        print(f"Cannot open Inflect model at {args.model_dir!r}: {exc}", file=sys.stderr)
        return 4

    try:
        sample_rate, waveform = tts.synthesize(
            args.text,
            speed=args.speed,
            variation=args.variation,
            seed=args.seed,
        )
    except Exception as exc:
        print(f"Inflect synthesis failed: {exc}", file=sys.stderr)
        return 5

    if waveform is None or len(waveform) == 0:
        print("Inflect returned an empty waveform.", file=sys.stderr)
        return 6

    import numpy as np

    wav = np.asarray(waveform, dtype=np.float32, order="C")
    if wav.ndim != 1:
        wav = wav.reshape(-1)

    # Normalize safety margin: Inflect outputs float audio in [-1, 1] in the
    # normal case. Clamp before converting to int16 so a single bad sample does
    # not clip the whole file into distortion.
    peak = float(np.abs(wav).max())
    if peak > 1.0:
        wav = wav / peak

    int16 = (wav * 32767.0).astype(np.int16, copy=False, order="C")

    try:
        import soundfile as sf

        sf.write(
            str(args.output),
            int16,
            int(sample_rate),
            subtype="PCM_16",
        )
    except Exception as exc:
        print(f"Cannot write WAV to {args.output!r}: {exc}", file=sys.stderr)
        return 7

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
