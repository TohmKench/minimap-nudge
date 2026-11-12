#!/usr/bin/env sh
# Regenerates tests/audio/sample.* (used by the audio decode test). Needs ffmpeg.
set -e
cd "$(dirname "$0")/.."
S=src-tauri/sounds/ping.wav
ffmpeg -loglevel error -y -i $S tests/audio/sample.mp3
ffmpeg -loglevel error -y -i $S -c:a libvorbis tests/audio/sample.ogg
ffmpeg -loglevel error -y -i $S tests/audio/sample.flac
ffmpeg -loglevel error -y -i $S -c:a aac tests/audio/sample.m4a
cp src-tauri/sounds/chime.wav tests/audio/sample.wav
