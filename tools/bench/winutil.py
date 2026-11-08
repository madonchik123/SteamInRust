"""Small Win32 helpers for the benchmark harness (ctypes only)."""
import ctypes
import ctypes.wintypes as wt
import time

user32 = ctypes.WinDLL("user32", use_last_error=True)
dwmapi = ctypes.WinDLL("dwmapi")
try:
    ctypes.WinDLL("shcore").SetProcessDpiAwareness(2)
except Exception:
    user32.SetProcessDPIAware()

HWND_TOPMOST = wt.HWND(-1)
HWND_NOTOPMOST = wt.HWND(-2)
SWP_NOSIZE, SWP_NOMOVE, SWP_NOACTIVATE, SWP_SHOWWINDOW = 0x1, 0x2, 0x10, 0x40
WM_CLOSE = 0x0010

EnumWindowsProc = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)


def window_pid(hwnd):
    pid = wt.DWORD()
    user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    return pid.value


def rect(hwnd):
    r = wt.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(r))
    return r.left, r.top, r.right, r.bottom


def title(hwnd):
    n = user32.GetWindowTextLengthW(hwnd)
    buf = ctypes.create_unicode_buffer(n + 1)
    user32.GetWindowTextW(hwnd, buf, n + 1)
    return buf.value


def cls(hwnd):
    buf = ctypes.create_unicode_buffer(256)
    user32.GetClassNameW(hwnd, buf, 256)
    return buf.value


def cloaked(hwnd):
    v = ctypes.c_int(0)
    dwmapi.DwmGetWindowAttribute(hwnd, 14, ctypes.byref(v), ctypes.sizeof(v))
    return v.value != 0


def windows(pids):
    """Visible, uncloaked top-level windows owned by any of `pids`."""
    out = []

    def cb(hwnd, _):
        if user32.IsWindowVisible(hwnd) and window_pid(hwnd) in pids and not cloaked(hwnd):
            l, t, r, b = rect(hwnd)
            if r - l > 50 and b - t > 50 and not user32.IsIconic(hwnd):
                out.append((hwnd, (l, t, r, b)))
        return True

    user32.EnumWindows(EnumWindowsProc(cb), 0)
    return out


def place(hwnd, x, y, w=None, h=None, topmost=True):
    flags = SWP_NOACTIVATE | SWP_SHOWWINDOW
    if w is None:
        flags |= SWP_NOSIZE
        w = h = 0
    user32.SetWindowPos(hwnd, HWND_TOPMOST if topmost else HWND_NOTOPMOST, x, y, w, h, flags)


def untop(hwnd):
    user32.SetWindowPos(hwnd, HWND_NOTOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE)


def close(hwnd):
    user32.PostMessageW(hwnd, WM_CLOSE, 0, 0)


def move_cursor(x, y):
    user32.SetCursorPos(int(x), int(y))


def click(x, y):
    move_cursor(x, y)
    time.sleep(0.03)
    user32.mouse_event(0x2, 0, 0, 0, 0)  # left down
    time.sleep(0.03)
    user32.mouse_event(0x4, 0, 0, 0, 0)  # left up


def scroll(x, y, notches):
    move_cursor(x, y)
    user32.mouse_event(0x800, 0, 0, ctypes.c_uint32(int(-120 * notches) & 0xFFFFFFFF).value, 0)


def key(vk, up=False):
    user32.keybd_event(vk, 0, 2 if up else 0, 0)


def tap(*vks):
    for v in vks:
        key(v)
    time.sleep(0.03)
    for v in reversed(vks):
        key(v, up=True)


def is_topmost(hwnd):
    return bool(user32.GetWindowLongW(hwnd, -20) & 0x8)


class LASTINPUTINFO(ctypes.Structure):
    _fields_ = [("cbSize", wt.UINT), ("dwTime", wt.DWORD)]


def last_input_tick():
    li = LASTINPUTINFO()
    li.cbSize = ctypes.sizeof(li)
    user32.GetLastInputInfo(ctypes.byref(li))
    return li.dwTime


user32.WindowFromPoint.argtypes = [wt.POINT]
user32.WindowFromPoint.restype = wt.HWND
user32.GetAncestor.restype = wt.HWND


def owner_pid_at(x, y):
    h = user32.WindowFromPoint(wt.POINT(int(x), int(y)))
    if not h:
        return None
    root = user32.GetAncestor(h, 2)  # GA_ROOT
    return window_pid(root or h)


def cursor():
    p = wt.POINT()
    user32.GetCursorPos(ctypes.byref(p))
    return p.x, p.y
