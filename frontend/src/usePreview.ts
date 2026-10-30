// frontend/src/usePreview.ts
// Strategy: HTMLAudioElement + asset protocol for immediate playback
// (streaming, no full decode), CACHED PEAKS for the waveform. On unsupported
// formats invoke a Rust command that decodes via symphonia in the background
// and returns float buffers — never block the UI thread.
import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { convertFileSrc } from "@tauri-apps/api/core";

export type Transport = {
  playing: boolean;
  position: number; // seconds
  duration: number; // seconds
  load: (path: string) => void;
  toggle: () => void;
  seek: (fraction: number) => void;
};

export function usePreview(): Transport {
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const [playing, setPlaying] = useState(false);
  const [position, setPosition] = useState(0);
  const [duration, setDuration] = useState(0);

  useEffect(() => {
    const el = new Audio();
    el.preload = "auto";
    audioRef.current = el;
    const onTime = () => setPosition(el.currentTime);
    const onMeta = () => setDuration(el.duration || 0);
    const onEnd = () => setPlaying(false);
    el.addEventListener("timeupdate", onTime);
    el.addEventListener("loadedmetadata", onMeta);
    el.addEventListener("ended", onEnd);
    return () => {
      el.pause();
      el.removeEventListener("timeupdate", onTime);
      el.removeEventListener("loadedmetadata", onMeta);
      el.removeEventListener("ended", onEnd);
    };
  }, []);

  return {
    playing,
    position,
    duration,
    load: (path: string) => {
      const el = audioRef.current;
      if (!el) return;
      el.src = convertFileSrc(path); // asset:// streaming — starts instantly
      el.play().then(() => setPlaying(true)).catch(() => setPlaying(false));
    },
    toggle: () => {
      const el = audioRef.current;
      if (!el) return;
      if (el.paused) {
        el.play().then(() => setPlaying(true)).catch(() => undefined);
      } else {
        el.pause();
        setPlaying(false);
      }
    },
    seek: (fraction: number) => {
      const el = audioRef.current;
      if (!el || !el.duration) return;
      el.currentTime = el.duration * Math.min(1, Math.max(0, fraction));
    },
  };
}

// Peaks come from the waveforms cache table (i16 min/max pairs, keyed by
// CONTENT id — duplicates share one decode, spec 5.2).
export async function drawWaveform(
  canvas: HTMLCanvasElement,
  contentId: number,
  positionFrac: number
) {
  const peaks = await invoke<number[]>("get_waveform", { contentId });
  const ctx = canvas.getContext("2d");
  if (!ctx || peaks.length < 2) return;
  const w = canvas.width;
  const h = canvas.height;
  const mid = h / 2;
  ctx.clearRect(0, 0, w, h);
  ctx.fillStyle = "rgba(233,236,242,0.75)";
  const pairs = peaks.length / 2;
  for (let x = 0; x < w; x++) {
    const i = Math.min(pairs - 1, Math.floor((x / w) * pairs));
    const min = peaks[i * 2] / 32768;
    const max = peaks[i * 2 + 1] / 32768;
    const y0 = mid + min * mid;
    const y1 = mid + max * mid;
    ctx.fillRect(x, Math.min(y0, y1), 1, Math.max(1, Math.abs(y1 - y0)));
  }
  // playhead
  ctx.fillStyle = "#ffb224";
  ctx.fillRect(Math.floor(w * positionFrac), 0, 1, h);
}
