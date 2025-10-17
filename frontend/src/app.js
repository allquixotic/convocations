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

// Console interception: funnel browser logs to backend with origin tags
function interceptConsole() {
  if (!window.__TAURI__?.event?.emit) {
    return; // Not in Tauri environment
  }

  const originalConsole = {
    log: console.log,
    error: console.error,
    warn: console.warn,
    debug: console.debug,
    info: console.info,
  };

  const createInterceptor = (level, original) => {
    return function (...args) {
      // Call original console method
      original.apply(console, args);

      // Format message
      const message = args
        .map((arg) => {
          if (typeof arg === 'object') {
            try {
              return JSON.stringify(arg);
            } catch {
              return String(arg);
            }
          }
          return String(arg);
        })
        .join(' ');

      // Emit to backend with origin tag
      window.__TAURI__.event.emit('frontend-log', {
        origin: 'frontend',
        level,
        message,
        timestamp: new Date().toISOString(),
      });
    };
  };

  console.log = createInterceptor('log', originalConsole.log);
  console.error = createInterceptor('error', originalConsole.error);
  console.warn = createInterceptor('warn', originalConsole.warn);
  console.debug = createInterceptor('debug', originalConsole.debug);
  console.info = createInterceptor('info', originalConsole.info);
}

// Install console interception early
interceptConsole();

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
    origin: payload.origin ?? 'backend',
  };
}

function App() {
  const [statusMessage, setStatusMessage] = useState('Starting backend…');
  const [baseUrl, setBaseUrl] = useState(null);
  const [health, setHealth] = useState(null);
  const [loadError, setLoadError] = useState(null);
  const [loading, setLoading] = useState(true);
  const [config, setConfig] = useState(null);
  const [ui, setUi] = useState({ theme: 'dark' });
  const [presets, setPresets] = useState([]);
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
  const [presetFormMode, setPresetFormMode] = useState(null); // null, 'create', or 'edit'
  const [editingPresetId, setEditingPresetId] = useState(null);
  const [presetFormData, setPresetFormData] = useState({
    id: '',
    name: '',
    weekday: 'saturday',
    timezone: 'America/New_York',
    start_time: '22:00',
    duration_minutes: 60,
    file_prefix: '',
    default_weeks_ago: 0,
  });
  const [presetError, setPresetError] = useState(null);

  const activeJobIdRef = useRef(null);
  useEffect(() => {
    activeJobIdRef.current = processingState.jobId;
  }, [processingState.jobId]);

  // Apply theme to document element
  useEffect(() => {
    const theme = ui?.theme?.toLowerCase() || 'dark';
    document.documentElement.setAttribute('data-theme', theme);
  }, [ui]);

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

        const fileConfig = settingsBody?.config ?? settingsBody;
        const runtimeConfig = fileConfig?.runtime ?? fileConfig;

        setHealth(healthBody);
        setConfig(deserializeConfig(runtimeConfig));
        setUi(fileConfig?.ui ?? { theme: 'dark' });
        setPresets(fileConfig?.presets ?? []);
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

  const handleThemeToggle = useCallback(() => {
    setUi((prev) => {
      const currentTheme = prev?.theme?.toLowerCase() || 'dark';
      const nextTheme = currentTheme === 'dark' ? 'light' : 'dark';
      return { ...prev, theme: nextTheme };
    });
  }, []);

  const handleSave = useCallback(async () => {
    if (!baseUrl || !config) {
      return;
    }
    try {
      setSaveState({ status: 'saving', message: null });

      // Build FileConfig with runtime (ephemeral), ui (persisted), and presets (persisted)
      const fileConfig = {
        schema_version: 1,
        runtime: normalizeConfigForApi(config),
        ui: ui,
        presets: presets,
      };

      const response = await fetch(`${baseUrl}/api/settings`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(fileConfig),
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
  }, [baseUrl, config, ui, presets]);

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

  const openCreatePresetForm = useCallback(() => {
    setPresetFormMode('create');
    setEditingPresetId(null);
    setPresetFormData({
      id: '',
      name: '',
      weekday: 'saturday',
      timezone: 'America/New_York',
      start_time: '22:00',
      duration_minutes: 60,
      file_prefix: '',
      default_weeks_ago: 0,
    });
    setPresetError(null);
  }, []);

  const openEditPresetForm = useCallback((preset) => {
    setPresetFormMode('edit');
    setEditingPresetId(preset.id);
    setPresetFormData({
      id: preset.id,
      name: preset.name,
      weekday: preset.weekday,
      timezone: preset.timezone,
      start_time: preset.start_time,
      duration_minutes: preset.duration_minutes,
      file_prefix: preset.file_prefix,
      default_weeks_ago: preset.default_weeks_ago,
    });
    setPresetError(null);
  }, []);

  const closePresetForm = useCallback(() => {
    setPresetFormMode(null);
    setEditingPresetId(null);
    setPresetError(null);
  }, []);

  const handlePresetFormChange = useCallback((field) => (event) => {
    const value = event.target.value;
    setPresetFormData((prev) => ({ ...prev, [field]: value }));
  }, []);

  const handlePresetFormNumberChange = useCallback((field) => (event) => {
    const value = event.target.value;
    const parsed = value === '' ? 0 : Math.max(0, Number.parseInt(value, 10) || 0);
    setPresetFormData((prev) => ({ ...prev, [field]: parsed }));
  }, []);

  const handleCreatePreset = useCallback(async () => {
    if (!baseUrl) {
      return;
    }
    try {
      setPresetError(null);
      const preset = {
        ...presetFormData,
        builtin: false,
      };
      const response = await fetch(`${baseUrl}/api/presets/create`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ preset }),
      });
      if (!response.ok) {
        let message = `Create failed (${response.status})`;
        try {
          const body = await response.json();
          if (body?.error) {
            message = body.error;
          }
        } catch (_) {
          // ignore
        }
        setPresetError(message);
        return;
      }
      const created = await response.json();
      setPresets((prev) => [...prev, created]);
      closePresetForm();
    } catch (err) {
      console.error('[Convocations] create preset failed', err);
      setPresetError(err.message ?? String(err));
    }
  }, [baseUrl, presetFormData, closePresetForm]);

  const handleUpdatePreset = useCallback(async () => {
    if (!baseUrl || !editingPresetId) {
      return;
    }
    try {
      setPresetError(null);
      const preset = {
        ...presetFormData,
        builtin: false,
      };
      const response = await fetch(`${baseUrl}/api/presets/update`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ id: editingPresetId, preset }),
      });
      if (!response.ok) {
        let message = `Update failed (${response.status})`;
        try {
          const body = await response.json();
          if (body?.error) {
            message = body.error;
          }
        } catch (_) {
          // ignore
        }
        setPresetError(message);
        return;
      }
      setPresets((prev) =>
        prev.map((p) => (p.id === editingPresetId ? { ...preset, id: presetFormData.id } : p))
      );
      closePresetForm();
    } catch (err) {
      console.error('[Convocations] update preset failed', err);
      setPresetError(err.message ?? String(err));
    }
  }, [baseUrl, editingPresetId, presetFormData, closePresetForm]);

  const handleDeletePreset = useCallback(async (presetId) => {
    if (!baseUrl) {
      return;
    }
    if (!confirm(`Delete preset "${presetId}"? This cannot be undone.`)) {
      return;
    }
    try {
      const response = await fetch(`${baseUrl}/api/presets/delete`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ id: presetId }),
      });
      if (!response.ok) {
        let message = `Delete failed (${response.status})`;
        try {
          const body = await response.json();
          if (body?.error) {
            message = body.error;
          }
        } catch (_) {
          // ignore
        }
        alert(message);
        return;
      }
      setPresets((prev) => prev.filter((p) => p.id !== presetId));
    } catch (err) {
      console.error('[Convocations] delete preset failed', err);
      alert(err.message ?? String(err));
    }
  }, [baseUrl]);

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
            'data-origin': entry.origin,
          },
          h(
            'div',
            { class: 'log-header' },
            h('span', { class: 'log-time' }, formatClockTime(new Date(entry.timestamp))),
            h('span', { class: 'log-origin', title: 'Origin' }, `[${entry.origin}]`),
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
      h(
        'div',
        { style: 'display: flex; justify-content: space-between; align-items: center;' },
        h(
          'div',
          null,
          h('h1', null, 'Convocations'),
          h('p', null, 'An Elder Scrolls Online chat log formatter'),
        ),
        h(
          'button',
          {
            type: 'button',
            class: 'button button--secondary',
            onClick: handleThemeToggle,
            title: 'Toggle theme',
            style: 'padding: 8px 12px;',
          },
          ui?.theme?.toLowerCase() === 'dark' ? '☀️ Light' : '🌙 Dark',
        ),
      ),
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
                  h('span', { class: 'field-label' }, 'Start Date & Time'),
                  h('input', {
                    type: 'datetime-local',
                    value: config.start ?? '',
                    onInput: handleText('start'),
                    disabled: eventSelection !== 'saturday',
                  }),
                  renderFieldMessages('start'),
                ),
                h(
                  'label',
                  { class: fieldClasses('field', 'end') },
                  h('span', { class: 'field-label' }, 'End Date & Time'),
                  h('input', {
                    type: 'datetime-local',
                    value: config.end ?? '',
                    onInput: handleText('end'),
                    disabled: eventSelection !== 'saturday',
                  }),
                  renderFieldMessages('end'),
                ),
                h(
                  'div',
                  {
                    class: fieldClasses(
                      'field duration-override-block',
                      ['one_hour', 'two_hours'],
                    ),
                  },
                  h(
                    'label',
                    { class: 'checkbox-field' },
                    h('input', {
                      type: 'checkbox',
                      checked: Boolean(config.one_hour || config.two_hours),
                      onChange: (event) => {
                        const checked = event.target.checked;
                        setConfig((prev) => {
                          if (!prev) return prev;
                          if (!checked) {
                            return { ...prev, one_hour: false, two_hours: false };
                          }
                          return { ...prev, one_hour: true, two_hours: false };
                        });
                      },
                    }),
                    h('span', null, 'Override duration'),
                  ),
                  (config.one_hour || config.two_hours) ? h(
                    'label',
                    { class: 'field', style: 'margin-top: 0.5rem;' },
                    h('span', { class: 'field-label' }, 'Duration (hours)'),
                    h('input', {
                      type: 'number',
                      min: 1,
                      value: config.two_hours ? 2 : 1,
                      onInput: (event) => {
                        const value = Number.parseInt(event.target.value, 10) || 1;
                        setConfig((prev) => {
                          if (!prev) return prev;
                          if (value === 1) {
                            return { ...prev, one_hour: true, two_hours: false };
                          } else if (value === 2) {
                            return { ...prev, one_hour: false, two_hours: true };
                          } else {
                            // For other values, just use one_hour flag for now
                            return { ...prev, one_hour: true, two_hours: false };
                          }
                        });
                      },
                    }),
                  ) : null,
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
                  { class: checkboxClasses('use_llm') },
                  h('input', {
                    type: 'checkbox',
                    checked: Boolean(config.use_llm),
                    onChange: handleCheckbox('use_llm'),
                  }),
                  h('span', null, 'Use AI corrections'),
                  renderFieldMessages('use_llm'),
                ),
                h(
                  'label',
                  {
                    class: checkboxClasses('keep_orig'),
                    style: config.use_llm ? '' : 'opacity: 0.5; pointer-events: none;',
                  },
                  h('input', {
                    type: 'checkbox',
                    checked: Boolean(config.keep_orig),
                    onChange: handleCheckbox('keep_orig'),
                    disabled: !config.use_llm,
                  }),
                  h('span', null, 'Keep original output (--keep-orig)'),
                  renderFieldMessages('keep_orig'),
                ),
                h(
                  'label',
                  {
                    class: checkboxClasses('no_diff'),
                    style: config.use_llm ? '' : 'opacity: 0.5; pointer-events: none;',
                  },
                  h('input', {
                    type: 'checkbox',
                    checked: Boolean(config.no_diff),
                    onChange: handleCheckbox('no_diff'),
                    disabled: !config.use_llm,
                  }),
                  h('span', null, 'Disable diff generation (--no-diff)'),
                  renderFieldMessages('no_diff'),
                ),
              ),
            ),
          ),
        )
      : null,
    config
      ? h(
          'section',
          { class: 'section-card' },
          h('h2', null, 'Preset Management'),
          h('p', { class: 'muted' }, 'Manage event presets for quick configuration'),
          presetFormMode === null
            ? h(
                'div',
                null,
                h(
                  'button',
                  {
                    type: 'button',
                    class: 'button button--secondary',
                    onClick: openCreatePresetForm,
                    style: 'margin-bottom: 1rem;',
                  },
                  'Create New Preset',
                ),
                h(
                  'div',
                  { class: 'preset-list' },
                  presets.length === 0
                    ? h('p', { class: 'muted' }, 'No presets available.')
                    : presets.map((preset) =>
                        h(
                          'div',
                          {
                            key: preset.id,
                            class: preset.builtin
                              ? 'preset-card preset-card--builtin'
                              : 'preset-card preset-card--user',
                          },
                          h(
                            'div',
                            { class: 'preset-header' },
                            h('h3', { class: 'preset-name' }, preset.name),
                            preset.builtin
                              ? h('span', { class: 'preset-badge' }, 'Built-in')
                              : h(
                                  'div',
                                  { class: 'preset-actions' },
                                  h(
                                    'button',
                                    {
                                      type: 'button',
                                      class: 'button button--small',
                                      onClick: () => openEditPresetForm(preset),
                                    },
                                    'Edit',
                                  ),
                                  h(
                                    'button',
                                    {
                                      type: 'button',
                                      class: 'button button--small button--danger',
                                      onClick: () => handleDeletePreset(preset.id),
                                    },
                                    'Delete',
                                  ),
                                ),
                          ),
                          h(
                            'div',
                            { class: 'preset-details' },
                            h('div', { class: 'preset-field' }, [
                              h('span', { class: 'preset-label' }, 'ID: '),
                              h('code', null, preset.id),
                            ]),
                            h('div', { class: 'preset-field' }, [
                              h('span', { class: 'preset-label' }, 'Weekday: '),
                              h('span', null, preset.weekday),
                            ]),
                            h('div', { class: 'preset-field' }, [
                              h('span', { class: 'preset-label' }, 'Time: '),
                              h('span', null, preset.start_time),
                            ]),
                            h('div', { class: 'preset-field' }, [
                              h('span', { class: 'preset-label' }, 'Duration: '),
                              h('span', null, `${preset.duration_minutes} minutes`),
                            ]),
                            h('div', { class: 'preset-field' }, [
                              h('span', { class: 'preset-label' }, 'File Prefix: '),
                              h('code', null, preset.file_prefix),
                            ]),
                            h('div', { class: 'preset-field' }, [
                              h('span', { class: 'preset-label' }, 'Default Weeks Ago: '),
                              h('span', null, preset.default_weeks_ago),
                            ]),
                            h('div', { class: 'preset-field' }, [
                              h('span', { class: 'preset-label' }, 'Timezone: '),
                              h('span', null, preset.timezone),
                            ]),
                          ),
                        ),
                      ),
                ),
              )
            : h(
                'div',
                { class: 'preset-form' },
                h('h3', null, presetFormMode === 'create' ? 'Create Preset' : 'Edit Preset'),
                presetError
                  ? h('p', { class: 'status-error' }, presetError)
                  : null,
                h(
                  'form',
                  {
                    onSubmit: (event) => event.preventDefault(),
                    class: 'config-form',
                  },
                  h(
                    'label',
                    { class: 'field' },
                    h('span', { class: 'field-label' }, 'Preset ID (unique identifier)'),
                    h('input', {
                      type: 'text',
                      value: presetFormData.id,
                      onInput: handlePresetFormChange('id'),
                      placeholder: 'e.g., custom-event-1',
                      required: true,
                    }),
                  ),
                  h(
                    'label',
                    { class: 'field' },
                    h('span', { class: 'field-label' }, 'Display Name'),
                    h('input', {
                      type: 'text',
                      value: presetFormData.name,
                      onInput: handlePresetFormChange('name'),
                      placeholder: 'e.g., Custom Event',
                      required: true,
                    }),
                  ),
                  h(
                    'label',
                    { class: 'field' },
                    h('span', { class: 'field-label' }, 'Weekday'),
                    h(
                      'select',
                      {
                        value: presetFormData.weekday,
                        onChange: handlePresetFormChange('weekday'),
                      },
                      ['monday', 'tuesday', 'wednesday', 'thursday', 'friday', 'saturday', 'sunday'].map((day) =>
                        h('option', { key: day, value: day }, day.charAt(0).toUpperCase() + day.slice(1)),
                      ),
                    ),
                  ),
                  h(
                    'label',
                    { class: 'field' },
                    h('span', { class: 'field-label' }, 'Start Time (HH:MM)'),
                    h('input', {
                      type: 'time',
                      value: presetFormData.start_time,
                      onInput: handlePresetFormChange('start_time'),
                      required: true,
                    }),
                  ),
                  h(
                    'label',
                    { class: 'field' },
                    h('span', { class: 'field-label' }, 'Duration (minutes)'),
                    h('input', {
                      type: 'number',
                      min: 1,
                      value: presetFormData.duration_minutes,
                      onInput: handlePresetFormNumberChange('duration_minutes'),
                      required: true,
                    }),
                  ),
                  h(
                    'label',
                    { class: 'field' },
                    h('span', { class: 'field-label' }, 'File Prefix'),
                    h('input', {
                      type: 'text',
                      value: presetFormData.file_prefix,
                      onInput: handlePresetFormChange('file_prefix'),
                      placeholder: 'e.g., event',
                      required: true,
                    }),
                  ),
                  h(
                    'label',
                    { class: 'field' },
                    h('span', { class: 'field-label' }, 'Default Weeks Ago'),
                    h('input', {
                      type: 'number',
                      min: 0,
                      value: presetFormData.default_weeks_ago,
                      onInput: handlePresetFormNumberChange('default_weeks_ago'),
                    }),
                  ),
                  h(
                    'label',
                    { class: 'field' },
                    h('span', { class: 'field-label' }, 'Timezone'),
                    h('input', {
                      type: 'text',
                      value: presetFormData.timezone,
                      onInput: handlePresetFormChange('timezone'),
                      placeholder: 'e.g., America/New_York',
                      required: true,
                    }),
                  ),
                  h(
                    'div',
                    { class: 'button-row', style: 'margin-top: 1rem;' },
                    h(
                      'button',
                      {
                        type: 'button',
                        class: 'button button--secondary',
                        onClick: closePresetForm,
                      },
                      'Cancel',
                    ),
                    h(
                      'button',
                      {
                        type: 'button',
                        class: 'button button--primary',
                        onClick: presetFormMode === 'create' ? handleCreatePreset : handleUpdatePreset,
                      },
                      presetFormMode === 'create' ? 'Create' : 'Update',
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
