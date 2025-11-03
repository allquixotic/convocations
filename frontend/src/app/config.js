import { CURATED_AUTO_VALUE } from './constants.js';

export function deserializeConfig(payload) {
  if (!payload) {
    return null;
  }
  const secretRef = payload.openrouter_api_key ?? null;
  return {
    ...payload,
    openrouter_api_key: undefined,
    openrouter_secret: secretRef,
    openrouter_has_secret: Boolean(secretRef),
    openrouter_key_input: '',
    chat_log_path: payload.chat_log_path ?? '',
    start: payload.start ?? '',
    end: payload.end ?? '',
    process_file: payload.process_file ?? '',
    outfile: payload.outfile ?? payload.outfile_override ?? '',
    output_directory: payload.output_directory ?? payload.output_directory_override ?? '',
    output_target:
      payload.output_target && typeof payload.output_target === 'string'
        ? payload.output_target
        : (payload.output_directory ?? payload.output_directory_override)
            && (payload.output_directory ?? payload.output_directory_override) !== ''
          ? 'directory'
          : 'file',
    openrouter_model:
      typeof payload.openrouter_model === 'string' && payload.openrouter_model.trim().length > 0
        ? payload.openrouter_model
        : CURATED_AUTO_VALUE,
  };
}

export function normalizeConfigForApi(config) {
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

  const target = config.output_target === 'directory' ? 'directory' : 'file';

  return {
    chat_log_path: trimOrNull(config.chat_log_path) ?? '',
    active_preset: trimOrNull(config.active_preset) ?? 'Saturday 10pm-midnight',
    weeks_ago: Number.isFinite(config.last) ? Number(config.last) : 0,
    dry_run: Boolean(config.dry_run),
    use_ai_corrections: Boolean(config.use_llm),
    keep_original_output: Boolean(config.keep_orig),
    show_diff: Boolean(!config.no_diff),
    cleanup_enabled: Boolean(config.cleanup),
    format_dialogue_enabled: Boolean(config.format_dialogue),
    outfile_override: trimOrNull(config.outfile),
    output_directory_override: trimOrNull(config.output_directory),
    output_target: target,
    duration_override: {
      enabled: Boolean(config.one_hour || config.two_hours),
      hours: config.two_hours ? 2.0 : config.one_hour ? 1.0 : 1.0,
    },
    openrouter_api_key: config.openrouter_secret ?? null,
    openrouter_model: trimOrNull(config.openrouter_model),
    free_models_only: Boolean(config.free_models_only),
  };
}
