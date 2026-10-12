#!/usr/bin/env python3
"""Live SDK -> hook -> HerdR smoke, confined to a newly named test session.

Requires built target/release/herdr-ahp and target/debug/examples/sdk_driver.
Never targets the inherited pane/socket. Keeps evidence in a private temp dir.
"""
import argparse
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import time
import uuid


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--interactions", action="store_true", help="also exercise optional permission events")
    args = parser.parse_args()
    if os.environ.get("HERDR_ENV") != "1":
        raise RuntimeError("Run from HerdR; no outside/focused-session fallback is permitted")
    herdr = shutil.which("herdr")
    if not herdr:
        raise RuntimeError("herdr is not on PATH")
    repo = Path(__file__).resolve().parent.parent
    bridge = repo / "target/release/herdr-ahp"
    driver = repo / "target/debug/examples/sdk_driver"
    if not bridge.is_file() or not driver.is_file():
        raise RuntimeError("Build release bridge and sdk_driver first")
    name = "herdr-ahp-e2e-" + uuid.uuid4().hex[:10]
    root = Path(tempfile.mkdtemp(prefix="herdr-ahp-live-"))
    # Scope every control call by --session; strip inherited caller context too.
    env = {k: v for k, v in os.environ.items() if not k.startswith("HERDR_")}
    prefix = [herdr, "--session", name]

    def cli(*words):
        result = subprocess.run(prefix + list(words), env=env, capture_output=True, text=True, timeout=15)
        if result.returncode:
            raise RuntimeError(result.stderr[:1500])
        return json.loads(result.stdout) if result.stdout.strip() else None

    def wait(check, message):
        deadline = time.monotonic() + 12
        while time.monotonic() < deadline:
            value = check()
            if value:
                return value
            time.sleep(0.05)
        raise RuntimeError(message)

    status = cli("status", "--json")["server"]
    socket = status["socket"]
    if status["running"] or socket == os.environ.get("HERDR_SOCKET_PATH") or ("/sessions/" + name + "/") not in socket:
        raise RuntimeError("Refusing non-isolated or already running session")
    if Path(socket).parent.exists():
        raise RuntimeError("Refusing an existing stopped session directory")
    evidence = {"session": name, "socket_is_isolated": True, "checks": []}
    started = False
    fifo_fd = None
    try:
        with (root / "server.log").open("w") as log:
            server = subprocess.Popen(prefix + ["server"], env=env, stdout=log, stderr=log, start_new_session=True)
        started = True
        evidence["server_pid"] = server.pid
        wait(lambda: cli("status", "--json")["server"]["running"], "Named headless server failed to start")
        evidence["version"] = cli("status", "--json")["server"]["version"]
        workspace = cli("workspace", "create", "--cwd", str(repo), "--label", "herdr-ahp-isolated-validation", "--no-focus")
        pane_id = workspace["result"]["root_pane"]["pane_id"]
        evidence["pane"] = pane_id

        def pane():
            return cli("pane", "get", pane_id)["result"]["pane"]

        bridge_args = ["--source", "org.example.herdr-ahp.live-sdk", "--agent", "sdk-smoke-agent",
                       "--event-source", "urn:example:agent"]
        resume_args = ["--", "sh", "-c", "printf resumed-sdk-smoke-{session_id}"]
        sample = "registration-with-interactions.json" if args.interactions else "registration.json"
        registration = json.loads((repo / "examples" / sample).read_text())
        registration["hooks"][0]["transport"].update(command=str(bridge), args=bridge_args + resume_args)
        registration_path = root / "registration.json"
        registration_path.write_text(json.dumps(registration))
        fifo = root / "events.fifo"
        os.mkfifo(fifo)
        fifo_fd = os.open(fifo, os.O_RDWR | os.O_NONBLOCK)
        # Run the demo as the foreground command in the test pane. After actual
        # driver exit, source-scoped release runs while sleep keeps the shell busy,
        # so idle-shell cleanup cannot substitute for the exit release assertion.
        q = shlex.quote
        command = (f"{q(str(driver))} {q(str(registration_path))} < {q(str(fifo))} "
                   f"> {q(str(root / 'acks.jsonl'))} 2> {q(str(root / 'driver.stderr'))}; "
                   f"driver_rc=$?; {shlex.join([str(bridge)] + bridge_args + ['--release'])}; "
                   f"release_rc=$?; printf '%s %s\\n' \"$driver_rc\" \"$release_rc\" > {q(str(root / 'driver-exit'))}; sleep 30")
        cli("pane", "run", pane_id, "sh -c " + q(command))
        count = 0

        def emit(kind, session="live-session-1"):
            nonlocal count
            event = {"type": kind, "session": {"id": session}, "turn": {"id": "live-turn-1"}}
            if kind == "session.start":
                event.update(harness={"name": "sdk-demo-host", "version": "0.1.0"}, items=[], permissionMode="default", trigger="startup")
            elif kind == "turn.start":
                event.update(items=[], trigger="user")
            elif kind == "turn.end":
                event.update(items=[], outcome="completed", continuationCount=0)
            else:
                event.update(call={"id": "live-call-1"}, tool={"name": "Shell", "kind": "shell", "input": {}, "origin": "native"}, path="shell")
                if kind.endswith("request"):
                    event.update(sandboxBypass=False, suggestions=[])
                else:
                    event.update(decision="allow", decidedBy="user")
            os.write(fifo_fd, (json.dumps(event) + "\n").encode())
            count += 1
            acks = root / "acks.jsonl"
            wait(lambda: acks.exists() and len(acks.read_text().splitlines()) >= count, "SDK acknowledgement missing; inspect driver.stderr")

        steps = [("session.start", "idle"), ("turn.start", "working")]
        if args.interactions:
            steps += [("tool.permission.request", "blocked"), ("tool.permission.resolved", "working")]
        steps += [("turn.end", "idle")]
        for kind, expected in steps:
            emit(kind)
            def accepted():
                current = pane()
                states = {"idle", "done"} if expected == "idle" else {expected}
                return current if current.get("agent") == "sdk-smoke-agent" and current.get("agent_status") in states else None
            current = wait(accepted, "State did not match " + kind)
            evidence["checks"].append({"event": kind, "agent": current["agent"], "state": current["agent_status"]})
        emit("session.start", "live-session-2")
        time.sleep(0.35)  # Allow bounded CLI reporting to finish before host shutdown.
        current = pane()
        assert current.get("agent") == "sdk-smoke-agent" and current.get("agent_status") in {"idle", "done"}
        evidence["checks"].append({"event": "session.start/session-switch", "agent": current["agent"], "state": current["agent_status"]})
        os.write(fifo_fd, b'{"shutdown":true}\n')
        os.close(fifo_fd)
        fifo_fd = None
        wait(lambda: (root / "driver-exit").exists(), "Demo host exit/release missing")
        assert (root / "driver-exit").read_text().strip() == "0 0", "Driver or release command failed"
        released = pane()
        assert not released.get("agent"), "Source ownership was not released"
        process_info = cli("pane", "process-info", "--pane", pane_id)["result"]["process_info"]
        assert any(process["name"] == "sleep" for process in process_info.get("foreground_processes", [])), "Idle-shell cleanup could mask release"
        evidence["release_foreground_processes"] = [process["name"] for process in process_info["foreground_processes"]]
        evidence["released_state"] = released["agent_status"]
        evidence["sdk_acknowledgements"] = [json.loads(line) for line in (root / "acks.jsonl").read_text().splitlines()]
        assert [ack["delivered"] for ack in evidence["sdk_acknowledgements"]] == list(range(1, count + 1))
        evidence["passed"] = True
    except Exception as error:
        evidence["failure"] = str(error)
        raise
    finally:
        if fifo_fd is not None:
            os.close(fifo_fd)
        cleanup_failed = False
        if started:
            for operation in ["stop", "delete"]:
                try:
                    result = subprocess.run(prefix + ["session", operation, name, "--json"], env=env, capture_output=True, text=True, timeout=15)
                    evidence["cleanup_" + operation] = {"exit": result.returncode, "stdout": result.stdout.strip(), "stderr": result.stderr.strip()}
                    cleanup_failed = cleanup_failed or result.returncode != 0
                except Exception as error:
                    evidence["cleanup_" + operation] = {"exit": None, "error": str(error)}
                    cleanup_failed = True
            try:
                server.wait(timeout=5)
            except subprocess.TimeoutExpired:
                evidence["server_reap"] = "Owned server did not exit after named cleanup; no process was killed directly"
                cleanup_failed = True
        if cleanup_failed:
            evidence["passed"] = False
        (root / "evidence.json").write_text(json.dumps(evidence, indent=2))
        print(json.dumps({"session": name, "evidence": str(root / "evidence.json"), "passed": evidence.get("passed", False), "checks": evidence["checks"], "cleanup": {k: v["exit"] for k, v in evidence.items() if k.startswith("cleanup_")}}, indent=2))
        if cleanup_failed:
            raise RuntimeError("Named test-resource cleanup incomplete; inspect retained evidence")


if __name__ == "__main__":
    main()
