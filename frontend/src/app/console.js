export function interceptConsole() {
  if (!window.__TAURI__?.event?.emit) {
    return;
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
      original.apply(console, args);

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
