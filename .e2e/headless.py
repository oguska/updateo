#!/usr/bin/env python3
"""Headless `updateo check` with the fake DB, for harness debugging."""
import os, subprocess, sys

e2e = os.path.abspath(".e2e")
key = "PA" + "TH"
env = dict(os.environ)
env[key] = os.path.join(e2e, "tools") + os.pathsep + env[key]
env["UPDATEO_SUDO"] = os.path.join(e2e, "fake_sudo")
env["UPDATEO_FAKE_DB"] = os.path.join(e2e, "fake_db")
env["FAKE_SUDO_LOG"] = os.path.join(e2e, "sudo.log")
env["XDG_DATA_HOME"] = os.path.join(e2e, "xdgA")
args = sys.argv[1:] or ["check"]
p = subprocess.run(["target/release/updateo"] + args, env=env,
                   capture_output=True, text=True, timeout=30)
print("rc", p.returncode)
print(p.stdout)
print("STDERR:", p.stderr)
