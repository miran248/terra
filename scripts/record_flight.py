#!/usr/bin/env python3
"""Record real gameplay scenes, join them with dissolves, and close the video loop."""
from pathlib import Path
import argparse
import json
import os
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
FPS = 30
DISSOLVE = 0.6


def ffmpeg(*args):
    subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", *map(str, args)], check=True)


def encode(frames, output, scratch):
    scenes = [line.split("\t", 2) for line in (frames / "scenes.tsv").read_text().splitlines()]
    clips = []
    durations = []
    preview_starts = []
    for index, count, title in scenes:
        count = int(count)
        prefix = f"{int(index):02}"
        expected = {f"{prefix}-{frame:05}.png" for frame in range(count)}
        if {p.name for p in frames.glob(f"{prefix}-*.png")} != expected:
            raise SystemExit(f"Incomplete capture: {title}")
        # Every retained shot comes from the same uninterrupted physical journey.
        # Compress uneventful ground/air transits; retain interactions and maneuvers.
        cuts = []
        beats = [(0, "Opening")]
        beats += [(int(frame), beat) for frame, beat in
                  (line.split("\t") for line in (frames / "beats.tsv").read_text().splitlines())]
        for (start, beat), (end, _) in zip(beats, beats[1:]):
            if beat in ("Drive", "DriveHome") and end - start > 35 * FPS:
                cuts.append((start + 8 * FPS, end - 8 * FPS))
            elif beat == "Walk" and end - start > 18 * FPS:
                cuts.append((start + 6 * FPS, end - 6 * FPS))
            elif beat == "ReturnToCar" and end - start > 35 * FPS:
                cuts.append((start + 5 * FPS, end - 5 * FPS))
            elif beat in ("Fly", "ApproachRunway") and end - start > 24 * FPS:
                cuts.append((start + 8 * FPS, end - 8 * FPS))
        ranges = []
        start = 0
        for left, right in cuts:
            ranges.append((start, left))
            start = right
        ranges.append((start, count))
        def edited_time(frame):
            removed = sum(max(0, min(frame, right) - left) for left, right in cuts)
            dissolves = sum(frame >= right for _, right in cuts)
            return (frame - removed) / FPS - dissolves * DISSOLVE
        first_beats = {}
        for frame, beat in beats:
            first_beats.setdefault(beat, frame)
        # Keep README stills reproducible and tied to the same gameplay capture.
        gallery = {
            "settlement.png": first_beats["Walk"] + FPS,
            "driving.png": first_beats["Drive"] + 16 * FPS,
            "mountain-flight.png": first_beats["Roll"] + 12 * FPS,
            "planet-map.png": 3 * FPS,
        }
        for name, frame in gallery.items():
            shutil.copyfile(frames / f"{prefix}-{frame:05}.png", output / name)
        preview_starts = [edited_time(first_beats[beat] + offset * FPS)
                          for beat, offset in (("Walk", 3), ("Drive", 5), ("Takeoff", 10), ("Roll", 4))]
        for start, end in ranges:
            clip = scratch / f"clip-{len(clips):02}.mp4"
            print(f"Encoding {title}: {start / FPS:.1f}–{end / FPS:.1f}s…", flush=True)
            ffmpeg("-framerate", FPS, "-start_number", start,
                   "-i", frames / f"{prefix}-%05d.png", "-frames:v", end - start,
                   "-c:v", "libx264", "-preset", "fast", "-crf", "16", "-pix_fmt", "yuv420p", clip)
            clips.append(clip)
            durations.append((end - start) / FPS)
    inputs = []
    for clip in clips:
        inputs.extend(["-i", clip])
    # The last dissolve ends on the exact opening still; no hard jump on repeat.
    inputs.extend(["-loop", "1", "-framerate", FPS, "-t", "0.7", "-i", frames / "00-00000.png"])
    filters = [f"[{i}:v]settb=1/{FPS},setpts=PTS-STARTPTS[v{i}]" for i in range(len(clips) + 1)]
    duration = durations[0]
    previous = "v0"
    for i in range(1, len(clips)):
        offset = duration - DISSOLVE
        filters.append(f"[{previous}][v{i}]xfade=transition=fade:duration={DISSOLVE}:offset={offset:.6f}[x{i}]")
        duration += durations[i] - DISSOLVE
        previous = f"x{i}"
    filters.append(f"[{previous}][v{len(clips)}]xfade=transition=fade:duration=0.6:offset={duration - 0.7:.6f}[loop]")
    video = output / "flight-showcase.mp4"
    ffmpeg(*inputs, "-filter_complex_threads", "1", "-filter_complex", ";".join(filters),
           "-map", "[loop]", "-frames:v", round(duration * FPS), "-r", FPS,
           "-c:v", "libx264", "-preset", "slow", "-crf", "23", "-pix_fmt", "yuv420p",
           "-movflags", "+faststart", "-an", video)
    # Reuse the opening IDR packet as the final frame. Re-encoding the same PNG
    # at two positions can differ under lossy H.264 even with identical inputs.
    opening = scratch / "opening.mp4"
    ffmpeg("-i", video, "-map", "0:v:0", "-frames:v", 1, "-c", "copy", opening)
    joined = scratch / "loop.mp4"
    manifest = scratch / "loop.txt"
    manifest.write_text(f"file '{video.resolve()}'\nfile '{opening.resolve()}'\n")
    ffmpeg("-f", "concat", "-safe", 0, "-i", manifest, "-c", "copy", "-movflags", "+faststart", joined)
    shutil.copyfile(joined, video)
    duration += 1 / FPS
    hashes = subprocess.check_output([
        "ffmpeg", "-v", "error", "-i", str(video), "-map", "0:v:0",
        "-f", "framemd5", "-"], text=True)
    decoded = [line.rsplit(",", 1)[1].strip() for line in hashes.splitlines() if not line.startswith("#")]
    assert decoded[0] == decoded[-1], "Loop endpoints must decode to identical pixels"
    # A compact montage covers exploration, weather and flight in the README.
    starts = preview_starts
    parts = [f"[0:v]trim=start={start:.3f}:duration=2,setpts=PTS-STARTPTS[p{i}]" for i, start in enumerate(starts)]
    parts.append("[p0][p1][p2][p3]concat=n=4:v=1:a=0,fps=12,scale=480:-1:flags=lanczos,split[a][b]")
    parts.append("[a]palettegen=max_colors=128[p];[b][p]paletteuse=dither=bayer:bayer_scale=3")
    ffmpeg("-i", video, "-filter_complex", ";".join(parts), "-loop", "0", output / "flight-preview.gif")
    info = json.loads(subprocess.check_output([
        "ffprobe", "-v", "error", "-select_streams", "v:0", "-show_entries",
        "stream=width,height,r_frame_rate,nb_frames,duration", "-of", "json", str(video),
    ]))["streams"][0]
    assert (info["width"], info["height"], info["r_frame_rate"], int(info["nb_frames"])) == (1280, 720, "30/1", round(duration * FPS))
    black_check = subprocess.run([
        "ffmpeg", "-hide_banner", "-nostats", "-i", str(video),
        "-vf", "blackdetect=d=0:pix_th=0.01:pic_th=0.99", "-an", "-f", "null", "-",
    ], capture_output=True, text=True, check=True)
    if "black_start:" in black_check.stderr:
        raise SystemExit("The recording contains black frames; inspect the capture before publishing")
    print(json.dumps(info, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--frames", type=Path, help="encode an existing full capture instead of launching the game")
    args = parser.parse_args()
    for tool in ("cargo", "ffmpeg", "ffprobe"):
        if shutil.which(tool) is None:
            raise SystemExit(f"Install {tool} before recording the showcase")
    output = ROOT / "docs/media"
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="terra-showcase-") as directory:
        scratch = Path(directory)
        frames = args.frames.resolve() if args.frames else scratch / "frames"
        if not args.frames:
            env = os.environ.copy()
            for name in ("TERRA_FLIGHT_PREVIEW", "TERRA_ASSET_SHOWCASE", "TERRA_PRODUCTION_CAPTURE"):
                env.pop(name, None)
            env["TERRA_FLIGHT_CAPTURE"] = str(frames)
            print("Recording exploration, vehicle interactions, flight, weather and the map…", flush=True)
            with (scratch / "capture.log").open("w") as log:
                result = subprocess.run(["cargo", "run", "-p", "main", "--features", "asset-review"],
                                        cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT)
            if result.returncode:
                raise SystemExit((scratch / "capture.log").read_text()[-8000:])
        encode(frames, output, scratch)
    print(f"Saved loopable showcase and preview to {output}")


if __name__ == "__main__":
    main()
