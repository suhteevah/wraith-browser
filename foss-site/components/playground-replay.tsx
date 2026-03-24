'use client';

import { useState, useEffect, useRef, useCallback } from 'react';
import type { SessionRecording, SessionStep } from '@/lib/replay-parser';

const SPEED_OPTIONS = [0.5, 1, 2] as const;

function StepRenderer({ step }: { step: SessionStep }) {
  if (step.type === 'command') {
    return (
      <div className="font-mono text-sm">
        <span className="text-emerald-400 select-none">$ </span>
        <span className="text-emerald-300">{step.content}</span>
        {step.annotation && (
          <div className="mt-1 ml-4 text-xs text-zinc-500 italic">
            # {step.annotation}
          </div>
        )}
      </div>
    );
  }

  if (step.type === 'output') {
    return (
      <div className="font-mono text-sm">
        <pre className="text-zinc-200 whitespace-pre-wrap break-words leading-relaxed">
          {step.content}
        </pre>
      </div>
    );
  }

  // annotation
  return (
    <div className="border-l-2 border-blue-500 bg-blue-500/10 rounded-r-md px-4 py-2 text-sm text-blue-300">
      {step.content}
    </div>
  );
}

export default function PlaygroundReplay({
  recording,
}: {
  recording: SessionRecording;
}) {
  const [currentStep, setCurrentStep] = useState(0);
  const [autoPlay, setAutoPlay] = useState(false);
  const [speed, setSpeed] = useState<(typeof SPEED_OPTIONS)[number]>(1);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const terminalRef = useRef<HTMLDivElement>(null);

  const totalSteps = recording.steps.length;
  const visibleSteps = recording.steps.slice(0, currentStep + 1);
  const isFinished = currentStep >= totalSteps - 1;

  const clearTimer = useCallback(() => {
    if (timerRef.current !== null) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  }, []);

  const advance = useCallback(() => {
    setCurrentStep((prev) => {
      if (prev >= totalSteps - 1) return prev;
      return prev + 1;
    });
  }, [totalSteps]);

  // Auto-play effect
  useEffect(() => {
    clearTimer();
    if (!autoPlay || isFinished) return;

    const step = recording.steps[currentStep];
    const delay = step.delay_ms / speed;

    timerRef.current = setTimeout(() => {
      advance();
    }, delay);

    return clearTimer;
  }, [autoPlay, currentStep, isFinished, speed, recording.steps, advance, clearTimer]);

  // Stop auto-play when finished
  useEffect(() => {
    if (isFinished) setAutoPlay(false);
  }, [isFinished]);

  // Scroll terminal to bottom when new steps appear
  useEffect(() => {
    if (terminalRef.current) {
      terminalRef.current.scrollTop = terminalRef.current.scrollHeight;
    }
  }, [currentStep]);

  const handleReset = () => {
    clearTimer();
    setAutoPlay(false);
    setCurrentStep(0);
  };

  return (
    <div className="rounded-xl border border-zinc-800 overflow-hidden bg-zinc-950">
      {/* Toolbar */}
      <div className="flex items-center justify-between px-4 py-2 bg-zinc-900 border-b border-zinc-800">
        <div className="flex items-center gap-3">
          {/* Window dots */}
          <div className="flex gap-1.5">
            <span className="w-3 h-3 rounded-full bg-red-500/70" />
            <span className="w-3 h-3 rounded-full bg-yellow-500/70" />
            <span className="w-3 h-3 rounded-full bg-green-500/70" />
          </div>
          <span className="text-xs text-zinc-400 font-mono">
            wraith-browser
          </span>
        </div>
        <div className="text-xs text-zinc-500">
          Step {currentStep + 1} of {totalSteps}
        </div>
      </div>

      {/* Terminal output */}
      <div
        ref={terminalRef}
        className="p-4 min-h-[240px] max-h-[480px] overflow-y-auto space-y-3"
      >
        {visibleSteps.map((step, i) => (
          <StepRenderer key={i} step={step} />
        ))}
        {!isFinished && (
          <span className="inline-block w-2 h-4 bg-emerald-400 animate-pulse" />
        )}
      </div>

      {/* Controls */}
      <div className="flex items-center justify-between px-4 py-3 bg-zinc-900/60 border-t border-zinc-800">
        <div className="flex items-center gap-2">
          <button
            onClick={advance}
            disabled={isFinished}
            className="px-3 py-1.5 text-xs font-medium rounded-md bg-emerald-600 text-white hover:bg-emerald-500 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
          >
            Next
          </button>
          <button
            onClick={() => setAutoPlay((p) => !p)}
            disabled={isFinished}
            className={`px-3 py-1.5 text-xs font-medium rounded-md transition-colors ${
              autoPlay
                ? 'bg-amber-600 text-white hover:bg-amber-500'
                : 'bg-zinc-700 text-zinc-200 hover:bg-zinc-600'
            } disabled:opacity-40 disabled:cursor-not-allowed`}
          >
            {autoPlay ? 'Pause' : 'Auto-play'}
          </button>
          <button
            onClick={handleReset}
            className="px-3 py-1.5 text-xs font-medium rounded-md bg-zinc-700 text-zinc-200 hover:bg-zinc-600 transition-colors"
          >
            Reset
          </button>
        </div>

        {/* Speed selector */}
        <div className="flex items-center gap-1.5">
          <span className="text-xs text-zinc-500">Speed:</span>
          {SPEED_OPTIONS.map((s) => (
            <button
              key={s}
              onClick={() => setSpeed(s)}
              className={`px-2 py-1 text-xs rounded-md transition-colors ${
                speed === s
                  ? 'bg-emerald-600 text-white'
                  : 'bg-zinc-800 text-zinc-400 hover:bg-zinc-700'
              }`}
            >
              {s}x
            </button>
          ))}
        </div>
      </div>

      {/* Progress bar */}
      <div className="h-1 bg-zinc-900">
        <div
          className="h-full bg-emerald-500 transition-all duration-300"
          style={{ width: `${((currentStep + 1) / totalSteps) * 100}%` }}
        />
      </div>
    </div>
  );
}
