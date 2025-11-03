function formatClockTime(date = new Date()) {
  return date.toLocaleTimeString([], {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  });
}

function formatMs(value) {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    return null;
  }
  if (value >= 1000) {
    return `${(value / 1000).toFixed(1)}s`;
  }
  return `${value.toFixed(0)}ms`;
}

function describeProgressEvent(payload) {
  switch (payload.kind) {
    case 'queued':
      return 'Job queued';
    case 'stage-begin':
      return payload.stage ? `Started ${payload.stage}` : 'Stage started';
    case 'stage-end': {
      const base = payload.stage ? `Finished ${payload.stage}` : 'Stage completed';
      const delta = formatMs(payload.stage_elapsed_ms);
      return delta ? `${base} (Δ ${delta})` : base;
    }
    case 'info':
      return payload.message ?? 'Update';
    case 'completed':
      return payload.message ?? 'Processing completed';
    case 'failed':
      return payload.error
        ? `Processing failed: ${payload.error}`
        : 'Processing failed';
    case 'diff':
      return payload.message ?? 'Diff generated';
    default:
      return payload.message ?? payload.kind ?? 'Update';
  }
}

function buildProgressEntry(payload) {
  const base = describeProgressEvent(payload);
  const elapsed = formatMs(payload.elapsed_ms);
  const pieces = [base];
  if (elapsed) {
    pieces.push(`t=${elapsed}`);
  }
  return {
    id: `${payload.job_id}-${payload.kind}-${Date.now()}-${Math.random()
      .toString(36)
      .slice(2, 6)}`,
    jobId: payload.job_id,
    kind: payload.kind,
    stage: payload.stage ?? null,
    message: pieces.join(' · '),
    error: payload.error ?? null,
    timestamp: new Date().toISOString(),
    origin: payload.origin ?? 'backend',
    diff: payload.diff ?? null,
  };
}

export { buildProgressEntry, describeProgressEvent, formatClockTime };
