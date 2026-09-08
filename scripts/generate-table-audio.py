#!/usr/bin/env python3
"""Reproducible, original PCM sound assets; no third-party recordings or music."""
import math
from pathlib import Path
import random
import struct
import wave

destination = Path(__file__).resolve().parents[1] / 'apps/web/public/table-audio/v1'
destination.mkdir(parents=True, exist_ok=True)
rate = 22050
recipes = {
    'card': (0.18, [320, 480]),
    'damage': (0.32, [130, 94]),
    'heal': (0.48, [523.25, 659.25, 783.99]),
    'resource': (0.24, [880, 1320]),
    'victory': (1.2, [261.63, 329.63, 392, 523.25]),
    'defeat': (0.95, [196, 155.56, 130.81]),
    'ambient': (8.0, [110, 165, 220]),
}
for name, (duration, notes) in recipes.items():
    random_source = random.Random(30)
    samples = []
    for index in range(round(rate * duration)):
        t = index / rate
        if name == 'ambient':
            envelope = 0.07 * (1 - math.cos(2 * math.pi * t / duration))
            sample = sum(math.sin(2 * math.pi * note * t) for note in notes) / len(notes)
        else:
            envelope = min(1, t / 0.008) * max(0, 1 - t / duration) ** 2 * 0.35
            note = notes[min(len(notes) - 1, int(t / duration * len(notes)))]
            sample = math.sin(2 * math.pi * note * t) * 0.8 + math.sin(2 * math.pi * note * 2 * t) * 0.2
            if name in ('card', 'damage'):
                sample = sample * 0.45 + (random_source.random() * 2 - 1) * 0.55
        samples.append(struct.pack('<h', round(max(-1, min(1, sample * envelope)) * 32767)))
    with wave.open(str(destination / f'{name}.wav'), 'wb') as output:
        output.setnchannels(1)
        output.setsampwidth(2)
        output.setframerate(rate)
        output.writeframes(b''.join(samples))
