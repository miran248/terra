#!/usr/bin/env python3
"""Generate candidates through the running Blender MCP add-on, never Blender CLI."""
import argparse
import json
from pathlib import Path
import socket
import tempfile

ROOT = Path(__file__).resolve().parents[3]
DEFAULT_OUTPUT = ROOT / "crates/main/assets/models/candidates"


def execute(code):
    # This is the installed lab_blender_org.mcp add-on's local transport:
    # NUL-terminated JSON, also used by execute_blender_code in the MCP server.
    request = {"type": "execute", "code": code, "strict_json": True}
    with socket.create_connection(("127.0.0.1", 9876), timeout=10) as connection:
        connection.settimeout(180)
        connection.sendall(json.dumps(request).encode() + b"\0")
        response = bytearray()
        while b"\0" not in response:
            chunk = connection.recv(65536)
            if not chunk:
                raise RuntimeError("Blender MCP closed before returning a result")
            response.extend(chunk)
    result = json.loads(response.split(b"\0", 1)[0])
    if result.get("status") != "ok":
        raise RuntimeError(result.get("message", str(result)))
    return result["result"]


def generate(destination):
    script = Path(__file__).with_name("pilot.py")
    code = (f"import runpy\n"
            f"pilot = runpy.run_path({str(script)!r})\n"
            f"result = pilot['generate']({str(destination)!r})\nresult")
    return execute(code)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out-dir", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--check", action="store_true", help="Regenerate in a temporary directory and compare; never writes output")
    args = parser.parse_args()
    output = args.out_dir.resolve()
    if args.check:
        with tempfile.TemporaryDirectory(prefix="terra-blender-check-") as temporary:
            generate(temporary)
            generated = Path(temporary)
            for name in sorted(p.name for p in generated.iterdir()):
                if not (output / name).exists() or (output / name).read_bytes() != (generated / name).read_bytes():
                    raise SystemExit(f"Candidate differs: {output / name}")
        print("Candidate GLBs and manifest are byte-identical")
    else:
        print(json.dumps(generate(output), indent=2))


if __name__ == "__main__":
    main()
