#!/usr/bin/env python3
"""Reduce gst-discoverer output to a bounded, path-free synthetic A/V probe."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path


MAX_INPUT_BYTES = 1_000_000
DURATION_RE = re.compile(r"^\s*Duration:\s*(\d+):(\d{2}):(\d{2})\.(\d{1,9})\s*$", re.MULTILINE)
CONTAINER_RE = re.compile(r"^\s*container #\d+:\s*video/webm\s*$", re.MULTILINE)
VIDEO_RE = re.compile(r"^\s*video #\d+:\s*video/x-vp8,([^\n]+)$", re.MULTILINE)
AUDIO_RE = re.compile(r"^\s*audio #\d+:\s*audio/x-opus,([^\n]+)$", re.MULTILINE)
STREAM_HEADER_RE = re.compile(
    r"^\s*([A-Za-z][A-Za-z0-9_-]*) #\d+:\s*[^\n]+$", re.MULTILINE
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    return parser.parse_args()


def read_input(path: Path) -> str:
    if path.is_symlink():
        fail("refusing symlinked discoverer input")
    try:
        raw = path.read_bytes()
    except OSError as error:
        fail(f"could not read discoverer input: {error}")
    if not raw or len(raw) > MAX_INPUT_BYTES or b"\x00" in raw:
        fail("discoverer output is empty, oversized, or contains NUL")
    try:
        return raw.decode("utf-8")
    except UnicodeDecodeError:
        fail("discoverer output is not UTF-8")


def parse_duration(text: str) -> int:
    matches = DURATION_RE.findall(text)
    if len(matches) != 1:
        fail("discoverer output must contain exactly one duration")
    hours, minutes, seconds, fraction = matches[0]
    if int(minutes) >= 60 or int(seconds) >= 60:
        fail("discoverer duration fields are invalid")
    fraction_ns = int(fraction.ljust(9, "0"))
    duration_ns = (
        ((int(hours) * 60 + int(minutes)) * 60 + int(seconds)) * 1_000_000_000
        + fraction_ns
    )
    if not 1_900_000_000 <= duration_ns <= 2_100_000_000:
        fail(f"synthetic duration is outside its budget: {duration_ns}ns")
    return duration_ns


def caps_field(caps: str, name: str, kind: str) -> int:
    caps = caps.strip()
    match = re.search(rf"(?:^|,\s*){re.escape(name)}=\({re.escape(kind)}\)(\d+)(?:,|$)", caps)
    if match is None:
        fail(f"missing {name} in synthetic caps")
    return int(match.group(1))


def fraction_field(caps: str, name: str) -> tuple[int, int]:
    caps = caps.strip()
    match = re.search(rf"(?:^|,\s*){re.escape(name)}=\(fraction\)(\d+)/(\d+)(?:,|$)", caps)
    if match is None or int(match.group(2)) == 0:
        fail(f"missing or invalid {name} in synthetic caps")
    return int(match.group(1)), int(match.group(2))


def inspect(text: str) -> dict[str, object]:
    stream_kinds = [kind.lower() for kind in STREAM_HEADER_RE.findall(text)]
    if sorted(stream_kinds) != ["audio", "container", "video"]:
        fail(
            "synthetic artifact stream topology must be exactly one container, "
            "one video, and one audio stream"
        )
    if len(CONTAINER_RE.findall(text)) != 1:
        fail("synthetic artifact is not exactly one WebM container")
    video = VIDEO_RE.findall(text)
    audio = AUDIO_RE.findall(text)
    if len(video) != 1 or len(audio) != 1:
        fail("synthetic artifact must contain exactly one VP8 and one Opus stream")
    width = caps_field(video[0], "width", "int")
    height = caps_field(video[0], "height", "int")
    frame_rate_numerator, frame_rate_denominator = fraction_field(video[0], "framerate")
    sample_rate = caps_field(audio[0], "rate", "int")
    channels = caps_field(audio[0], "channels", "int")
    if (width, height, frame_rate_numerator, frame_rate_denominator) != (320, 180, 30, 1):
        fail("synthetic video caps differ from the audited profile")
    if (sample_rate, channels) != (48_000, 1):
        fail("synthetic audio caps differ from the audited profile")
    if not re.search(r"^\s*Seekable:\s*yes\s*$", text, re.MULTILINE):
        fail("synthetic artifact is not seekable")
    return {
        "schema_version": 1,
        "container": "video/webm",
        "duration_ns": parse_duration(text),
        "seekable": True,
        "video": {
            "codec": "vp8",
            "width": width,
            "height": height,
            "frame_rate_numerator": frame_rate_numerator,
            "frame_rate_denominator": frame_rate_denominator,
        },
        "audio": {"codec": "opus", "sample_rate": sample_rate, "channels": channels},
        "result": "pass",
    }


def write_evidence(path: Path, evidence: dict[str, object]) -> None:
    if path.exists() and path.is_symlink():
        fail("refusing symlinked discoverer evidence output")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def fail(message: str) -> None:
    print(f"GStreamer media probe failed: {message}", file=sys.stderr)
    raise SystemExit(1)


def main() -> None:
    args = parse_args()
    evidence = inspect(read_input(args.input))
    write_evidence(args.evidence, evidence)
    print("GStreamer media probe passed (VP8 320x180@30 + Opus 48kHz mono)")


if __name__ == "__main__":
    main()
