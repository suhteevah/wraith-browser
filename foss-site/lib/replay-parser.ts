export interface SessionStep {
  type: 'command' | 'output' | 'annotation';
  content: string;
  delay_ms: number;
  annotation?: string;
}

export interface SessionRecording {
  title: string;
  description: string;
  steps: SessionStep[];
}

/**
 * Validates a raw JSON object as a SessionRecording.
 * Returns the typed recording or throws on invalid input.
 */
export function parseRecording(raw: unknown): SessionRecording {
  if (typeof raw !== 'object' || raw === null) {
    throw new Error('Recording must be a JSON object');
  }

  const obj = raw as Record<string, unknown>;

  if (typeof obj.title !== 'string' || obj.title.length === 0) {
    throw new Error('Recording must have a non-empty title');
  }
  if (typeof obj.description !== 'string') {
    throw new Error('Recording must have a description');
  }
  if (!Array.isArray(obj.steps) || obj.steps.length === 0) {
    throw new Error('Recording must have at least one step');
  }

  const validTypes = new Set(['command', 'output', 'annotation']);

  const steps: SessionStep[] = obj.steps.map((s: unknown, i: number) => {
    if (typeof s !== 'object' || s === null) {
      throw new Error(`Step ${i} must be an object`);
    }
    const step = s as Record<string, unknown>;

    if (typeof step.type !== 'string' || !validTypes.has(step.type)) {
      throw new Error(`Step ${i} has invalid type: ${String(step.type)}`);
    }
    if (typeof step.content !== 'string') {
      throw new Error(`Step ${i} must have string content`);
    }
    if (typeof step.delay_ms !== 'number' || step.delay_ms < 0) {
      throw new Error(`Step ${i} must have a non-negative delay_ms`);
    }

    return {
      type: step.type as SessionStep['type'],
      content: step.content,
      delay_ms: step.delay_ms,
      annotation: typeof step.annotation === 'string' ? step.annotation : undefined,
    };
  });

  return {
    title: obj.title,
    description: obj.description as string,
    steps,
  };
}
