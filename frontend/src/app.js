import { h, render } from 'preact';
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'preact/hooks';

const apiBasePromise = window.__TAURI__?.core?.invoke('get_api_base_url');
const VALIDATION_DEBOUNCE_MS = 250;
const DEFAULT_STATUS = 'Idle';

function deserializeConfig(payload) {
  if (!payload) {
    return null;
  }
  return {
    ...payload,
    start: payload.start ?? '',
    end: payload.end ?? '',
    process_file: payload.process_file ?? '',
    outfile: payload.outfile ?? '',
  };
}

function normalizeConfigForApi(config) {
  if (!config) {
    return null;
  }
  const trimOrNull = (value) => {
    if (typeof value !== 'string') {
      return value ?? null;
    }
    const trimmed = value.trim();
    return trimmed.length === 0 ? null : trimmed;
  };

  return {
    last: Number.isFinite(config.last) ? Number(config.last) : 0,
    dry_run: Boolean(config.dry_run),
    infile: trimOrNull(config.infile) ?? '',
    start: trimOrNull(config.start),
    end: trimOrNull(config.end),
    rsm7: Boolean(config.rsm7),
    rsm8: Boolean(config.rsm8),
    tp6: Boolean(config.tp6),
    one_hour: Boolean(config.one_hour),
    two_hours: Boolean(config.two_hours),
    process_file: trimOrNull(config.process_file),
    format_dialogue: Boolean(config.format_dialogue),
    cleanup: Boolean(config.cleanup),
    use_llm: Boolean(config.use_llm),
    keep_orig: Boolean(config.keep_orig),
    no_diff: Boolean(config.no_diff),
    outfile: trimOrNull(config.outfile),
  };
}

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
  };
}

function App() {
  const [statusMessage, setStatusMessage] = useState('Starting backend…');
  const [baseUrl, setBaseUrl] = useState(null);
  const [health, setHealth] = useState(null);
  const [loadError, setLoadError] = useState(null);
  const [loading, setLoading] = useState(true);
  const [config, setConfig] = useState(null);
  const [derived, setDerived] = useState({ outfile: null });
  const [configLoaded, setConfigLoaded] = useState(false);
  const [validation, setValidation] = useState(null);
  const [validationStatus, setValidationStatus] = useState('idle');
  const [saveState, setSaveState] = useState({ status: 'idle', message: null });
  const [processingState, setProcessingState] = useState({
    active: false,
    jobId: null,
    status: DEFAULT_STATUS,
    error: null,
  });
  const [progressLog, setProgressLog] = useState([]);

  const activeJobIdRef = useRef(null);
  useEffect(() => {
    activeJobIdRef.current = processingState.jobId;
  }, [processingState.jobId]);

  const outputPlaceholder = useMemo(() => {
    if (derived?.outfile?.default) {
      return `Default: ${derived.outfile.default}`;
    }
    return 'Optional custom output path';
  }, [derived]);

  const outfileHint = useMemo(() => {
    if (!derived?.outfile?.default) {
      return null;
    }
    return derived.outfile.overridden
      ? `Override active. Default would be ${derived.outfile.default}`
      : `Default output: ${derived.outfile.default}`;
  }, [derived]);

  useEffect(() => {
    let cancelled = false;

    async function bootstrap() {
      try {
        setStatusMessage('Waiting for backend…');
        const base = await apiBasePromise;
        if (!base) {
          throw new Error('API base URL not available');
        }
        if (cancelled) {
          return;
        }
        setBaseUrl(base);
        setStatusMessage('Fetching health & settings…');

        const [healthRes, settingsRes] = await Promise.all([
          fetch(`${base}/api/health`),
          fetch(`${base}/api/settings`),
        ]);

        if (!healthRes.ok) {
          throw new Error(`Health check failed (${healthRes.status})`);
        }
        if (!settingsRes.ok) {
          throw new Error(`Settings fetch failed (${settingsRes.status})`);
        }

        const healthBody = await healthRes.json();
        const settingsBody = await settingsRes.json();

        if (cancelled) {
          return;
        }

        const configPayload = settingsBody?.config ?? settingsBody;

        setHealth(healthBody);
        setConfig(deserializeConfig(configPayload));
        setDerived({
          outfile: settingsBody?.outfile ?? null,
        });
        setConfigLoaded(true);
        setStatusMessage('Connected to Convocations REST API');
        setLoadError(null);
      } catch (err) {
        if (!cancelled) {
          console.error('[Convocations] bootstrap failed', err);
          setLoadError(err.message ?? String(err));
          setStatusMessage('Unable to contact backend');
        }
      } finally {
        if (!cancelled) {
          setLoading(false);
        }
      }
    }

    bootstrap();

    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!window.__TAURI__?.event) {
      return undefined;
    }

    let cleanup = null;
    window.__TAURI__.event
      .listen('process-progress', ({ payload }) => {
        if (!payload || typeof payload !== 'object') {
          return;
        }

        setProgressLog((prev) => {
          const entry = buildProgressEntry(payload);
          const next = [...prev, entry];
          return next.length > 200 ? next.slice(next.length - 200) : next;
        });

        setProcessingState((prev) => {
          const jobId = payload.job_id;
          if (!jobId) {
            return prev;
          }

          if (prev.jobId && prev.jobId !== jobId && payload.kind !== 'queued') {
            return prev;
          }

          switch (payload.kind) {
            case 'queued':
              return {
                active: true,
                jobId,
                status: 'Queued job…',
                error: null,
              };
            case 'stage-begin':
              return {
                active: true,
                jobId,
                status: payload.stage
                  ? `Running: ${payload.stage}`
                  : 'Processing…',
                error: null,
              };
            case 'stage-end':
              return {
                active: true,
                jobId,
                status: payload.stage
                  ? `Completed ${payload.stage}`
                  : 'Processing…',
                error: null,
              };
            case 'completed':
              return {
                active: false,
                jobId,
                status: 'Completed successfully',
                error: null,
              };
            case 'failed':
              return {
                active: false,
                jobId,
                status: 'Failed',
                error:
                  payload.error ??
                  payload.message ??
                  'Processing failed unexpectedly',
              };
            case 'info':
              return {
                active: prev.active,
                jobId,
                status: payload.message ?? prev.status,
                error: null,
              };
            default:
              return prev;
          }
        });
      })
      .then((fn) => {
        cleanup = fn;
      })
      .catch((err) => {
        console.error('[Convocations] failed to register progress listener', err);
      });

    return () => {
      if (cleanup) {
        cleanup();
      }
    };
  }, []);

  useEffect(() => {
    if (!baseUrl || !config || !configLoaded) {
      return undefined;
    }

    const controller = new AbortController();
    setValidationStatus('loading');

    const timer = setTimeout(async () => {
      try {
        const payload = normalizeConfigForApi(config);
        const response = await fetch(`${baseUrl}/api/validate`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(payload),
          signal: controller.signal,
        });
        if (!response.ok) {
          throw new Error(`Validation failed (${response.status})`);
        }
        const body = await response.json();
        setValidation(body);
        if (body?.outfile) {
          setDerived((prev) => {
            const current = prev?.outfile ?? null;
            if (
              current &&
              current.default === body.outfile.default &&
              current.effective === body.outfile.effective &&
              current.overridden === body.outfile.overridden
            ) {
              return prev;
            }
            return { ...prev, outfile: body.outfile };
          });
        }
        setValidationStatus('ready');
      } catch (err) {
        if (controller.signal.aborted) {
          return;
        }
        console.error('[Convocations] validation failed', err);
        setValidationStatus('error');
      }
    }, VALIDATION_DEBOUNCE_MS);

    return () => {
      controller.abort();
      clearTimeout(timer);
    };
  }, [baseUrl, config, configLoaded]);

  const eventSelection = useMemo(() => {
    if (!config) {
      return 'saturday';
    }
    if (config.rsm7) {
      return 'rsm7';
    }
    if (config.rsm8) {
      return 'rsm8';
    }
    if (config.tp6) {
      return 'tp6';
    }
    return 'saturday';
  }, [config]);

  const applyEventSelection = useCallback((value) => {
    setConfig((prev) => {
      if (!prev) {
        return prev;
      }
      return {
        ...prev,
        rsm7: value === 'rsm7',
        rsm8: value === 'rsm8',
        tp6: value === 'tp6',
      };
    });
  }, []);

  const handleCheckbox = useCallback(
    (field) => (event) => {
      const checked = event.target.checked;
      setConfig((prev) => (prev ? { ...prev, [field]: checked } : prev));
    },
    [],
  );

  const handleDurationToggle = useCallback(
    (field) => (event) => {
      const checked = event.target.checked;
      setConfig((prev) => {
        if (!prev) {
          return prev;
        }
        if (!checked) {
          return { ...prev, [field]: false };
        }
        if (field === 'one_hour') {
          return { ...prev, one_hour: true, two_hours: false };
        }
        if (field === 'two_hours') {
          return { ...prev, one_hour: false, two_hours: true };
        }
        return prev;
      });
    },
    [],
  );

  const handleText = useCallback(
    (field) => (event) => {
      const value = event.target.value;
      setConfig((prev) => (prev ? { ...prev, [field]: value } : prev));
    },
    [],
  );

  const handleNumber = useCallback(
    (field) => (event) => {
      const value = event.target.value;
      const parsed =
        value === '' ? 0 : Math.max(0, Number.parseInt(value, 10) || 0);
      setConfig((prev) => (prev ? { ...prev, [field]: parsed } : prev));
    },
    [],
  );

  const handleSave = useCallback(async () => {
    if (!baseUrl || !config) {
      return;
    }
    try {
      setSaveState({ status: 'saving', message: null });
      const payload = normalizeConfigForApi(config);
      const response = await fetch(`${baseUrl}/api/settings`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload),
      });
      if (!response.ok) {
        throw new Error(`Save failed (${response.status})`);
      }
      setSaveState({
        status: 'saved',
        message: `Saved at ${formatClockTime()}`,
      });
    } catch (err) {
      console.error('[Convocations] save failed', err);
      setSaveState({
        status: 'error',
        message: err.message ?? String(err),
      });
    }
  }, [baseUrl, config]);

  const handleProcess = useCallback(async () => {
    if (!baseUrl || !config) {
      return;
    }

    if (processingState.active) {
      return;
    }

    if (validation && validation.valid === false) {
      setProcessingState({
        active: false,
        jobId: null,
        status: 'Resolve validation errors before processing.',
        error: null,
      });
      return;
    }

    const payload = normalizeConfigForApi(config);
    setProcessingState({
      active: true,
      jobId: null,
      status: 'Submitting job…',
      error: null,
    });
    setProgressLog([]);

    try {
      const response = await fetch(`${baseUrl}/api/process`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload),
      });

      if (!response.ok) {
        let message = `Process failed (${response.status})`;
        try {
          const body = await response.json();
          if (body?.error) {
            message = body.error;
          }
        } catch (_) {
          // ignore JSON parse failure
        }
        if (response.status === 409) {
          message = 'A processing job is already running.';
        }
        setProcessingState({
          active: false,
          jobId: null,
          status: 'Idle',
          error: message,
        });
        return;
      }

      const body = await response.json();
      const jobId = body?.job_id ?? null;
      setProcessingState({
        active: true,
        jobId,
        status: 'Waiting for backend progress…',
        error: null,
      });
    } catch (err) {
      console.error('[Convocations] process failed', err);
      setProcessingState({
        active: false,
        jobId: null,
        status: 'Idle',
        error: err.message ?? String(err),
      });
    }
  }, [baseUrl, config, processingState.active, validation]);

  const runDisabled =
    processingState.active ||
    validationStatus === 'loading' ||
    (validation && validation.valid === false);

  const eventOptions = [
    { value: 'saturday', label: 'Saturday Night (default)' },
    { value: 'rsm7', label: 'RSM7 – Tuesday 7PM ET' },
    { value: 'rsm8', label: 'RSM8 – Tuesday 8PM ET' },
    { value: 'tp6', label: 'TP6 – Friday 6PM ET' },
  ];

  const fieldErrors = validation?.field_errors ?? {};
  const fieldWarnings = validation?.field_warnings ?? {};

  const collectMessages = (fields, bucket) => {
    const keys = Array.isArray(fields) ? fields : [fields];
    const seen = new Set();
    const messages = [];
    for (const key of keys) {
      const raw = bucket?.[key];
      if (!raw) {
        continue;
      }
      const values = Array.isArray(raw) ? raw : [raw];
      for (const value of values) {
        if (value == null) {
          continue;
        }
        const text = String(value).trim();
        if (!text.length || seen.has(text)) {
          continue;
        }
        seen.add(text);
        messages.push(text);
      }
    }
    return messages;
  };

  const getFieldErrors = (fields) => collectMessages(fields, fieldErrors);
  const getFieldWarnings = (fields) => collectMessages(fields, fieldWarnings);
  const hasFieldError = (fields) => getFieldErrors(fields).length > 0;

  const fieldClasses = (base, fields) => {
    const classes = new Set(base.split(' ').filter(Boolean));
    if (hasFieldError(fields)) {
      classes.add('field--invalid');
    }
    if (getFieldWarnings(fields).length > 0) {
      classes.add('field--warning');
    }
    return Array.from(classes).join(' ');
  };

  const checkboxClasses = (fields) => {
    const classes = ['checkbox-field'];
    if (hasFieldError(fields)) {
      classes.push('checkbox-field--invalid');
    }
    if (getFieldWarnings(fields).length > 0) {
      classes.push('checkbox-field--warning');
    }
    return classes.join(' ');
  };

  const renderFieldMessages = (fields) => {
    const errors = getFieldErrors(fields);
    const warnings = getFieldWarnings(fields);
    if (errors.length === 0 && warnings.length === 0) {
      return null;
    }
    return h(
      'div',
      { class: 'field-messages' },
      errors.map((message, index) =>
        h(
          'p',
          {
            key: `error-${index}-${message}`,
            class: 'field-message field-message--error',
          },
          message,
        ),
      ),
      warnings.map((message, index) =>
        h(
          'p',
          {
            key: `warning-${index}-${message}`,
            class: 'field-message field-message--warning',
          },
          message,
        ),
      ),
    );
  };

  const renderProgressLog = () => {
    if (progressLog.length === 0) {
      return h('p', { class: 'muted' }, 'No activity yet. Start a run to see updates.');
    }

    return h(
      'ul',
      { class: 'progress-log' },
      progressLog.map((entry) =>
        h(
          'li',
          {
            key: entry.id,
            class:
              entry.kind === 'failed'
                ? 'log-entry log-entry--error'
                : entry.kind === 'completed'
                  ? 'log-entry log-entry--success'
                  : 'log-entry',
          },
          h(
            'div',
            { class: 'log-header' },
            h('span', { class: 'log-time' }, formatClockTime(new Date(entry.timestamp))),
            h('span', { class: 'log-kind' }, entry.kind),
            entry.jobId
              ? h('span', { class: 'log-job' }, `Job ${entry.jobId.slice(0, 8)}`)
              : null,
          ),
          h('p', { class: 'log-message' }, entry.message),
          entry.error
            ? h('p', { class: 'log-error-message' }, entry.error)
            : null,
        ),
      ),
    );
  };

  return h(
    'main',
    { class: 'container' },
    h(
      'section',
      { class: 'hero' },
      h('h1', null, 'Convocations'),
      h('p', null, 'An Elder Scrolls Online chat log formatter'),
    ),
    loading
      ? h(
          'section',
          { class: 'section-card' },
          h('p', null, 'Loading configuration…'),
        )
      : null,
    !loading && !config
      ? h(
          'section',
          { class: 'section-card' },
          h(
            'p',
            { class: 'status-error' },
            'Configuration unavailable. Check backend logs.',
          ),
        )
      : null,
    config
      ? h(
          'section',
          { class: 'section-card' },
          h('h2', null, 'Configuration'),
          h(
          'form',
          {
            class: 'config-form',
            onSubmit: (event) => event.preventDefault(),
          },
          validationStatus === 'loading' && configLoaded
            ? h(
                'div',
                { class: 'form-banner form-banner--pending' },
                'Validating configuration…',
              )
            : null,
          validationStatus === 'error'
            ? h(
                'div',
                { class: 'form-banner form-banner--error' },
                'Validation failed. Check console for details.',
              )
            : null,
          validation &&
          validationStatus !== 'loading' &&
          validation.valid === false
            ? h(
                'div',
                { class: 'form-banner form-banner--error' },
                'Resolve the inline validation errors below before running.',
              )
            : null,
          h(
            'div',
            { class: 'form-group' },
            h('h3', { class: 'group-title' }, 'Session Selection'),
            h(
                'div',
                { class: 'field-grid' },
                h(
                  'div',
                  {
                    class: fieldClasses('field field--full', [
                      'rsm7',
                      'rsm8',
                      'tp6',
                    ]),
                  },
                  h('span', { class: 'field-label' }, 'Event'),
                  h(
                    'div',
                    { class: 'radio-group' },
                    eventOptions.map((option) =>
                      h(
                        'label',
                        { key: option.value, class: 'radio-option' },
                        h('input', {
                          type: 'radio',
                          name: 'event-selection',
                          checked: eventSelection === option.value,
                          onChange: () => applyEventSelection(option.value),
                        }),
                        h('span', null, option.label),
                      ),
                    ),
                  ),
                  renderFieldMessages(['rsm7', 'rsm8', 'tp6']),
                ),
                h(
                  'label',
                  { class: fieldClasses('field', 'last') },
                  h('span', { class: 'field-label' }, 'Weeks Ago (--last)'),
                  h('input', {
                    type: 'number',
                    min: 0,
                    value: config.last ?? 0,
                    onInput: handleNumber('last'),
                  }),
                  renderFieldMessages('last'),
                ),
                h(
                  'label',
                  { class: fieldClasses('field', 'start') },
                  h('span', { class: 'field-label' }, 'Start (ISO8601)'),
                  h('input', {
                    type: 'text',
                    placeholder: 'YYYY-MM-DDTHH:MM',
                    value: config.start ?? '',
                    onInput: handleText('start'),
                  }),
                  renderFieldMessages('start'),
                ),
                h(
                  'label',
                  { class: fieldClasses('field', 'end') },
                  h('span', { class: 'field-label' }, 'End (ISO8601)'),
                  h('input', {
                    type: 'text',
                    placeholder: 'YYYY-MM-DDTHH:MM',
                    value: config.end ?? '',
                    onInput: handleText('end'),
                  }),
                  renderFieldMessages('end'),
                ),
                h(
                  'div',
                  {
                    class: fieldClasses(
                      'field checkbox-cluster',
                      ['one_hour', 'two_hours'],
                    ),
                  },
                  h('span', { class: 'field-label' }, 'Duration Overrides'),
                  h(
                    'label',
                    { class: checkboxClasses('one_hour') },
                    h('input', {
                      type: 'checkbox',
                      checked: Boolean(config.one_hour),
                      onChange: handleDurationToggle('one_hour'),
                    }),
                    h('span', null, 'Force 1 hour (--1h)'),
                  ),
                  h(
                    'label',
                    { class: checkboxClasses('two_hours') },
                    h('input', {
                      type: 'checkbox',
                      checked: Boolean(config.two_hours),
                      onChange: handleDurationToggle('two_hours'),
                    }),
                    h('span', null, 'Force 2 hours (--2h)'),
                  ),
                  renderFieldMessages(['one_hour', 'two_hours']),
                ),
              ),
            ),
            h(
              'div',
              { class: 'form-group' },
              h('h3', { class: 'group-title' }, 'Files & Modes'),
              h(
                'div',
                { class: 'field-grid' },
                h(
                  'label',
                  { class: fieldClasses('field field--full', 'infile') },
                  h('span', { class: 'field-label' }, 'ChatLog Path (--infile)'),
                  h('input', {
                    type: 'text',
                    value: config.infile ?? '',
                    onInput: handleText('infile'),
                  }),
                  renderFieldMessages('infile'),
                ),
                h(
                  'label',
                  { class: fieldClasses('field field--full', 'process_file') },
                  h(
                    'span',
                    { class: 'field-label' },
                    'Processed Input (--process-file)',
                  ),
                  h('input', {
                    type: 'text',
                    placeholder: 'Optional pre-filtered file',
                    value: config.process_file ?? '',
                    onInput: handleText('process_file'),
                  }),
                  renderFieldMessages('process_file'),
                ),
                h(
                  'label',
                  { class: fieldClasses('field field--full', 'outfile') },
                  h('span', { class: 'field-label' }, 'Output File (--outfile)'),
                  h('input', {
                    type: 'text',
                    placeholder: outputPlaceholder,
                    value: config.outfile ?? '',
                    onInput: handleText('outfile'),
                  }),
                  renderFieldMessages('outfile'),
                  outfileHint
                    ? h('span', { class: 'field-hint' }, outfileHint)
                    : null,
                ),
              ),
              h(
                'div',
                { class: 'checkbox-grid' },
                h(
                  'label',
                  { class: checkboxClasses('dry_run') },
                  h('input', {
                    type: 'checkbox',
                    checked: Boolean(config.dry_run),
                    onChange: handleCheckbox('dry_run'),
                  }),
                  h('span', null, 'Dry run (print command only)'),
                  renderFieldMessages('dry_run'),
                ),
              ),
            ),
            h(
              'div',
              { class: 'form-group' },
              h('h3', { class: 'group-title' }, 'Processing Options'),
              h(
                'div',
                { class: 'checkbox-grid' },
                h(
                  'label',
                  { class: checkboxClasses('format_dialogue') },
                  h('input', {
                    type: 'checkbox',
                    checked: Boolean(config.format_dialogue),
                    onChange: handleCheckbox('format_dialogue'),
                  }),
                  h('span', null, 'Format dialogue'),
                  renderFieldMessages('format_dialogue'),
                ),
                h(
                  'label',
                  { class: checkboxClasses('cleanup') },
                  h('input', {
                    type: 'checkbox',
                    checked: Boolean(config.cleanup),
                    onChange: handleCheckbox('cleanup'),
                  }),
                  h('span', null, 'Cleanup middle stage'),
                  renderFieldMessages('cleanup'),
                ),
                h(
                  'label',
                  { class: checkboxClasses('use_llm') },
                  h('input', {
                    type: 'checkbox',
                    checked: Boolean(config.use_llm),
                    onChange: handleCheckbox('use_llm'),
                  }),
                  h('span', null, 'Use LLM corrections'),
                  renderFieldMessages('use_llm'),
                ),
                h(
                  'label',
                  { class: checkboxClasses('keep_orig') },
                  h('input', {
                    type: 'checkbox',
                    checked: Boolean(config.keep_orig),
                    onChange: handleCheckbox('keep_orig'),
                  }),
                  h('span', null, 'Keep original output (--keep-orig)'),
                  renderFieldMessages('keep_orig'),
                ),
                h(
                  'label',
                  { class: checkboxClasses('no_diff') },
                  h('input', {
                    type: 'checkbox',
                    checked: Boolean(config.no_diff),
                    onChange: handleCheckbox('no_diff'),
                  }),
                  h('span', null, 'Disable diff generation (--no-diff)'),
                  renderFieldMessages('no_diff'),
                ),
              ),
            ),
          ),
        )
      : null,
    h(
      'section',
      { class: 'section-card' },
      h('h2', null, 'Actions'),
      h(
        'div',
        { class: 'button-row' },
        h(
          'button',
          {
            type: 'button',
            class: 'button button--secondary',
            onClick: handleSave,
            disabled: loading || !config,
          },
          saveState.status === 'saving' ? 'Saving…' : 'Save Settings',
        ),
        h(
          'button',
          {
            type: 'button',
            class: 'button button--primary',
            onClick: handleProcess,
            disabled: loading || !config || runDisabled,
          },
          processingState.active ? 'Processing…' : 'Run Processor',
        ),
      ),
      saveState.status === 'saved'
        ? h('p', { class: 'status-ok' }, saveState.message ?? 'Settings saved.')
        : null,
      saveState.status === 'error'
        ? h('p', { class: 'status-error' }, saveState.message ?? 'Save failed.')
        : null,
    ),
    h(
      'section',
      { class: 'section-card technical-log' },
      h('h2', null, 'Technical Log'),
      h(
        'div',
        { class: 'technical-status-grid' },
        h(
          'div',
          { class: 'technical-panel' },
          h('h3', { class: 'technical-panel__title' }, 'Backend'),
          h('p', { class: 'status-line' }, statusMessage),
          health
            ? h(
                'div',
                { class: 'technical-meta' },
                h('span', null, `Version ${health.version}`),
                baseUrl ? h('span', null, baseUrl) : null,
              )
            : null,
          loadError ? h('p', { class: 'status-error' }, loadError) : null,
        ),
        h(
          'div',
          { class: 'technical-panel' },
          h('h3', { class: 'technical-panel__title' }, 'Processing'),
          h('p', null, `Status: ${processingState.status}`),
          processingState.jobId
            ? h('p', { class: 'muted' }, `Job ID: ${processingState.jobId}`)
            : null,
          processingState.error
            ? h('p', { class: 'status-error' }, processingState.error)
            : null,
        ),
      ),
      h(
        'div',
        { class: 'technical-log-stream' },
        renderProgressLog(),
      ),
    ),
  );
}

render(h(App, null), document.getElementById('app'));
