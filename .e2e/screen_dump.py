#!/usr/bin/env python3
"""Deterministic PTY render probe with assertions.

Run the real app inside a pty, drive keys, rasterize the screen, then assert
the visible text. Exits 0 when every expectation holds, 1 otherwise.

usage: screen_dump.py XDG_STATE [keys] [--expect STR]... [--reject STR]...
       [--input-cell STR] [--width N] [--height N]

keys: literal chars plus <CR> <BS> <ESC> <UP> <DOWN> <TAB>, each key held
150ms apart so the 100ms tick can re-render between them.
"""
import fcntl, os, pty, re, select, struct, subprocess, sys, termios, time

KEYS = {"<CR>": b"\r", "<BS>": b"\x7f", "<ESC>": b"\x1b",
        "<UP>": b"\x1b[A", "<DOWN>": b"\x1b[B", "<TAB>": b"\t", "<SP>": b" ",
        "<BACKTAB>": b"\x1b[Z"}
# crossterm'in TTY olmayan çıkışta bastığı bilgi satırı
TAG = re.compile(rb"\bMouse Support - not a TTY\b")


class Screen:
    def __init__(self, w, h):
        self.w, self.h, self.cx, self.cy = w, h, 0, 0
        self.grid = [[" "] * w for _ in range(h)]

    def put(self, ch):
        o = ord(ch)
        if o == 13:
            self.cx = 0
        elif o == 10:
            self.cx = 0
            self.cy = min(self.cy + 1, self.h - 1)
        elif o == 8:
            self.cx = max(self.cx - 1, 0)
            self.grid[self.cy][self.cx] = " "
        elif o == 9:
            self.cx = min((self.cx // 8 + 1) * 8, self.w - 1)
        elif 32 <= o < 0x3000:
            if self.cx < self.w:
                self.grid[self.cy][self.cx] = ch
                self.cx += 1
        # other C1/C0 controls (e.g. soft wrap) are ignored

    def rows(self):
        return ["".join(r).rstrip() for r in self.grid]


def feed(sc, buf, pending=b""):
    """Rasterize bytes onto the grid; returns the trailing partial sequence."""
    buf = pending + buf
    i, n = 0, len(buf)
    while i < n:
        c = buf[i]
        if c == 0x1b:
            if i + 1 < n and buf[i + 1:i + 2] == b"[":
                j = i + 2
                while j < n and not (0x40 <= buf[j] < 0x40 + 0x40):
                    j += 1
                if j >= n:
                    return buf[i:]  # CSI split across reads: resume next chunk
                final, param = buf[j:j + 1], buf[i + 2:j]
                if final in (b"H", b"f"):
                    nums = [int(x) for x in param.split(b";") if x] or [1, 1]
                    if len(nums) == 1:
                        nums.append(1)
                    sc.cy, sc.cx = nums[0] - 1, nums[1] - 1
                elif final == b"J":
                    nums = [int(x) for x in param.split(b";") if x] or [0]
                    if nums[0] == 2:
                        sc.grid = [[" "] * sc.w for _ in range(sc.h)]
                        sc.cx = sc.cy = 0
                elif final == b"K":
                    for k in range(sc.cx, sc.w):
                        sc.grid[sc.cy][k] = " "
                i = j + 1
                continue
            i += 1
            continue
        if c == 0x0f or c == 0x0e:
            i += 1
            continue
        if c < 0x80:
            sc.put(chr(c))
            i += 1
            continue
        # decode one UTF-8 codepoint
        need = 2 if c < 0xE0 else 3 if c < 0xF0 else 4
        chunk = buf[i:i + need]
        if len(chunk) < need:
            return chunk  # split multibyte char
        try:
            text = chunk.decode("utf-8")
        except UnicodeDecodeError:
            i += 1
            continue
        for ch in text:
            sc.put(ch)
        i += need
    return b""


def keys_of(spec):
    out = []
    for tok in re.findall(r"<[A-Z]+>|.", spec):
        out.append(KEYS.get(tok, tok.encode()))
    return out


def main():
    argv = sys.argv[1:]
    xdg, spec = argv[0], argv[1] if len(argv) > 1 else ""
    width, height = 82, 24
    expects, log_lines, input_cell, sudo = [], [], None, None
    rejects = []
    rest = argv[2:]
    k = 0
    while k < len(rest):
        flag = rest[k]
        if flag == "--expect":
            expects.append(rest[k + 1]); k += 2
        elif flag == "--reject":
            rejects.append(rest[k + 1]); k += 2
        elif flag == "--log":
            log_lines.append(rest[k + 1]); k += 2
        elif flag == "--input-cell":
            input_cell = rest[k + 1]; k += 2
        elif flag == "--sudo":
            sudo = rest[k + 1]; k += 2
        elif flag in ("--width", "--height"):
            val = int(rest[k + 1])
            if flag == "--width":
                width = val
            else:
                height = val
            k += 2
        else:
            raise SystemExit("bilinmeyen arg: %s" % flag)

    e2e = os.path.abspath(".e2e")
    log = os.path.join(e2e, "sudo.log")
    os.makedirs(os.path.join(e2e, "logs"), exist_ok=True)
    for name in ("installed", "updates", "rdeps", "flat_installed",
                 "flat_updates"):
        open(os.path.join(e2e, "logs", name), "w").close()
    open(log, "w").close()
    env = dict(os.environ,
               TERM="xterm-256color",
               PATH=os.path.join(e2e, "tools") + ":" + os.environ["PATH"],
               UPDATEO_SUDO=sudo or os.path.join(e2e, "fake_sudo"),
               UPDATEO_FAKE_DB=os.path.join(e2e, "fake_db"),
               FAKE_SUDO_LOG=log,
               XDG_DATA_HOME=os.path.abspath(xdg))
    binary = os.path.abspath(os.path.join("target", "release", "updateo"))
    # winsize before exec: crossterm reads the terminal size once at startup
    m, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", height, width, 0, 0))
    pid = os.fork()
    if pid == 0:  # child: controlling terminal + raw TUI on the slave side
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
        for fd in (0, 1, 2):
            os.dup2(slave, fd)
        if m > 2:
            os.close(m)
        os.close(slave)
        os.execve(binary, [binary], env)
        os._exit(127)
    os.close(slave)
    pending = b""

    def pump(seconds):
        """Drain the pty for a fixed window; the app redraws every tick."""
        nonlocal pending
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            r, _, _ = select.select([m], [], [], 0.05)
            if not r:
                continue
            try:
                chunk = os.read(m, 65536)
            except OSError:
                return False
            if not chunk:
                return False
            if raw_out is not None:
                raw_out.write(chunk)
            pending = feed(sc, TAG.sub(b"", chunk), pending)
        return True

    sc = Screen(width, height)
    raw_out = open(os.path.join(e2e, "raw.bin"), "wb") \
        if os.environ.get("SCREEN_DUMP_RAW") else None
    # startup: the async pacman/flatpak checks have to finish first
    for _ in range(30):
        pump(0.4)
        if re.search(r"[1-9][0-9]*/[1-9]", "\n".join(sc.rows())):
            break
    else:
        print("FAIL list ekranı hiç dolmadı")
        print("\n".join(sc.rows()))
        os.kill(pid, 9)
        return 1
    pump(0.3)
    for b in keys_of(spec):
        try:
            os.write(m, b)
        except OSError:
            break
        pump(0.35)  # > 2 x 100ms tick: the screen has redrawn with the key
    pump(0.6)
    os.kill(pid, 9)
    os.waitpid(pid, 0)
    rows = sc.rows()
    flat = "\n".join(rows)

    if os.environ.get("SCREEN_DUMP_SHOW"):
        print("\n".join(rows))
        if os.path.exists(log):
            print("=== sudo.log ===")
            print(open(log).read().strip())

    failures = []
    for want in expects:
        if want not in flat:
            failures.append("beklenen metin yok: %r" % want)
    for unwanted in rejects:
        if unwanted in flat:
            failures.append("beklenmeyen metin var: %r" % unwanted)
    sudo_calls = open(log).read() if os.path.exists(log) else ""
    for want in log_lines:
        if want not in sudo_calls:
            failures.append("sudo.log %r yok (içerik: %r)" % (want, sudo_calls))
    if input_cell is not None:
        # girdi satırı: kısaltılmış ipucu "Enter/Esc" yalnız orada basılır
        line = next((r for r in rows if "Enter/Esc" in r), "")
        # hane ipucuna dek doldurulur: yazılan metin ipucunun solunda kalır
        typed = line.split("Enter/Esc")[0].rstrip()
        if not typed.endswith(input_cell):
            failures.append("girdi hanesi %r != beklenen %r (satır: %r)"
                            % (typed, input_cell, line.strip()))
    for f in failures:
        print("FAIL", f)
    print("OK" if not failures else "FAILED %d" % len(failures))
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
