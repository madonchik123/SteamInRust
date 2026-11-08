"""A black, borderless, always-on-top rectangle behind the window under test.
It hides whatever else is on the desktop from the capture region."""
import sys
import tkinter as tk
import ctypes

try:
    ctypes.WinDLL("shcore").SetProcessDpiAwareness(2)
except Exception:
    pass

x, y, w, h = map(int, sys.argv[1:5])
root = tk.Tk()
root.overrideredirect(True)
root.configure(bg="black")
root.geometry(f"{w}x{h}+{x}+{y}")
root.attributes("-topmost", True)
root.title("bench-backdrop")
if len(sys.argv) > 5:  # optional full-size picture (the "game" behind the overlay)
    img = tk.PhotoImage(file=sys.argv[5])
    tk.Label(root, image=img, bd=0).pack()

# A line on stdin turns the backdrop a barely-visible grey: the launch marker
# that lines the screen recording up with the harness clock.
import threading, queue
q = queue.Queue()
threading.Thread(target=lambda: [q.put(l) for l in sys.stdin], daemon=True).start()


def poll():
    while not q.empty():
        cmd = q.get().strip()
        root.configure(bg="#0c0c0c" if cmd == "mark" else "black")
    root.after(2, poll)


poll()
root.mainloop()
