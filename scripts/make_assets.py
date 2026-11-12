"""Generates the built-in alert sounds and the app/tray icons (all original)."""
import os, wave
import numpy as np
from PIL import Image, ImageDraw

ROOT = os.path.join(os.path.dirname(__file__), '..', 'assets')
SR = 44100

def env(n, attack=0.005, decay=4.0):
    t = np.arange(n) / SR
    a = np.minimum(1, t / attack)
    return a * np.exp(-decay * t)

def tone(freq, dur, decay=4.0, harmonics=((1, 1.0),), attack=0.005):
    n = int(SR * dur); t = np.arange(n) / SR
    sig = sum(amp * np.sin(2 * np.pi * freq * h * t) for h, amp in harmonics)
    return sig * env(n, attack, decay)

def place(total, parts):
    out = np.zeros(int(SR * total))
    for start, sig in parts:
        i = int(SR * start); out[i:i + len(sig)] += sig[: len(out) - i]
    return out

def write(name, sig, peak=0.85):
    sig = sig / (np.max(np.abs(sig)) or 1) * peak
    fade = int(SR * 0.01); sig[-fade:] *= np.linspace(1, 0, fade)
    data = (sig * 32767).astype('<i2')
    with wave.open(os.path.join(ROOT, 'sounds', name), 'wb') as w:
        w.setnchannels(1); w.setsampwidth(2); w.setframerate(SR); w.writeframes(data.tobytes())

bell = ((1, 1.0), (2.76, 0.35), (5.4, 0.12))
write('chime.wav', place(1.3, [(0, tone(880, 1.0, 4.5, bell)), (0.16, tone(1318.5, 1.1, 4.0, bell))]))

# sonar ping: sine with a slight downward glide and a faint echo
n = int(SR * 1.2); t = np.arange(n) / SR
f = 1500 * np.exp(-0.15 * t)
ping = np.sin(2 * np.pi * np.cumsum(f) / SR) * env(n, 0.003, 3.2)
write('ping.wav', place(1.6, [(0, ping), (0.32, ping * 0.25)]))

beep = tone(1046.5, 0.12, 8, ((1, 1), (3, 0.08)), attack=0.004)
write('double-beep.wav', place(0.45, [(0, beep), (0.17, beep)]))

def knock():
    n = int(SR * 0.18); t = np.arange(n) / SR
    body = np.sin(2 * np.pi * 180 * t) * np.exp(-30 * t)
    click = np.random.default_rng(7).normal(0, 1, n) * np.exp(-180 * t) * 0.4
    return body + click
write('knock.wav', place(0.5, [(0, knock()), (0.16, knock() * 0.9)]))

rise = np.concatenate([tone(f, 0.1, 10, ((1, 1), (2, 0.2)), 0.003) for f in (659.3, 880, 1174.7)])
write('rise.wav', place(0.5, [(0, rise[: int(SR * 0.1)]), (0.09, rise[int(SR * 0.1): int(SR * 0.2)]), (0.18, tone(1174.7, 0.3, 7, ((1, 1), (2, 0.2))))]))

# ---- icons: rounded square "minimap" with a radar ping
def icon(size, accent, bg='#0f1419', frame='#24303b'):
    s = 4  # supersample
    W = size * s
    im = Image.new('RGBA', (W, W), (0, 0, 0, 0)); d = ImageDraw.Draw(im)
    r = int(W * 0.2)
    d.rounded_rectangle([0, 0, W - 1, W - 1], r, fill=bg, outline=frame, width=max(1, W // 24))
    # minimap lanes (subtle diagonal + corner)
    lw = max(1, W // 22)
    d.line([(W * 0.18, W * 0.82), (W * 0.82, W * 0.18)], fill='#2c3a46', width=lw)
    d.line([(W * 0.18, W * 0.82), (W * 0.18, W * 0.18), (W * 0.82, W * 0.18)], fill='#2c3a46', width=lw)
    d.line([(W * 0.18, W * 0.82), (W * 0.82, W * 0.82), (W * 0.82, W * 0.18)], fill='#2c3a46', width=lw)
    # ping rings
    cx, cy = W * 0.5, W * 0.5
    for rad, wdt in ((0.34, 0.05), (0.21, 0.06)):
        R = W * rad
        d.ellipse([cx - R, cy - R, cx + R, cy + R], outline=accent, width=max(1, int(W * wdt)))
    R = W * 0.09
    d.ellipse([cx - R, cy - R, cx + R, cy + R], fill=accent)
    return im.resize((size, size), Image.LANCZOS)

icon(256, '#3ecfb2').save(os.path.join(ROOT, 'icon.png'))
icon(256, '#3ecfb2').save(os.path.join(ROOT, 'icon.ico'), sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
icon(64, '#3ecfb2').save(os.path.join(ROOT, 'tray-active.png'))
icon(64, '#8b98a5').save(os.path.join(ROOT, 'tray-idle.png'))
print('assets written')
