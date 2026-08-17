"""Local ARM64 execution worker for the x86-64 Android development AVD.

The worker does not simulate a successful result. Each `/probe` request starts
an ARM64 Debian container under Docker's QEMU user-mode emulation and runs the
cross-compiled Prisma probe inside it. The response is the evidence record
printed by the real loader -> translator -> ARM64 JIT -> Session path.
"""

from __future__ import annotations

import http.server
import subprocess
import threading
import uuid


HOST = "0.0.0.0"
PORT = 8765
PROBE_VOLUME = "prisma-arm64-target"
PROBE = "/target/aarch64-unknown-linux-gnu/debug/prisma-arm64-probe"
RUN_LOCK = threading.Lock()


def execute_probe() -> tuple[int, str]:
    container = f"prisma-arm64-probe-{uuid.uuid4().hex[:12]}"
    command = [
        "docker",
        "run",
        "--rm",
        "--name",
        container,
        "--platform",
        "linux/arm64",
        "-v",
        f"{PROBE_VOLUME}:/target:ro",
        "debian:bookworm-slim",
        PROBE,
    ]
    try:
        with RUN_LOCK:
            completed = subprocess.run(
                command,
                check=False,
                capture_output=True,
                text=True,
                timeout=180,
            )
        output = completed.stdout.strip()
        evidence = next(
            (line for line in reversed(output.splitlines()) if line.startswith("REAL|")),
            "",
        )
        if completed.returncode == 0 and evidence:
            return 200, evidence
        detail = (completed.stderr or output or "probe-exited-without-evidence").strip()
        return 500, f"FAILED|stage=arm64-worker|error={detail[:400]}"
    except subprocess.TimeoutExpired:
        return 504, "FAILED|stage=arm64-worker|error=timeout"
    finally:
        try:
            subprocess.run(
                ["docker", "rm", "-f", container],
                check=False,
                capture_output=True,
                timeout=15,
            )
        except (OSError, subprocess.TimeoutExpired):
            pass


class ProbeHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler API
        if self.path == "/health":
            self._reply(200, "ok")
            return
        if self.path != "/probe":
            self._reply(404, "not found")
            return
        status, body = execute_probe()
        self._reply(status, body)

    def _reply(self, status: int, body: str) -> None:
        payload = body.encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "text/plain; charset=utf-8")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def log_message(self, message_format: str, *args: object) -> None:
        print(f"arm64-worker: {message_format % args}")


def main() -> None:
    server = http.server.ThreadingHTTPServer((HOST, PORT), ProbeHandler)
    print(f"Prisma ARM64 worker listening on http://127.0.0.1:{PORT}")
    try:
        server.serve_forever()
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
