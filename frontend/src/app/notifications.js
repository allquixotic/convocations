import { h } from 'preact';
import { useCallback, useEffect, useRef, useState } from 'preact/hooks';

const DEFAULT_TIMEOUT = 8000;

export function useNotifications(options = {}) {
  const { defaultTimeout = DEFAULT_TIMEOUT } = options;
  const [notifications, setNotifications] = useState([]);
  const timersRef = useRef(new Map());

  const dismiss = useCallback((id) => {
    setNotifications((prev) => prev.filter((item) => item.id !== id));
    const timer = timersRef.current.get(id);
    if (timer) {
      window.clearTimeout(timer);
      timersRef.current.delete(id);
    }
  }, []);

  const notify = useCallback(
    ({ message, type = 'info', timeout, id }) => {
      const trimmed = typeof message === 'string' ? message.trim() : '';
      if (!trimmed.length) {
        return null;
      }
      const entryId = id ?? `notif-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;
      const expiryMs = timeout === undefined ? defaultTimeout : timeout;
      const entry = {
        id: entryId,
        message: trimmed,
        type,
        timeout: expiryMs,
      };

      setNotifications((prev) => [...prev, entry]);

      if (expiryMs && Number.isFinite(expiryMs) && expiryMs > 0) {
        const handle = window.setTimeout(() => dismiss(entryId), expiryMs);
        timersRef.current.set(entryId, handle);
      }

      return entryId;
    },
    [defaultTimeout, dismiss],
  );

  useEffect(() => {
    return () => {
      timersRef.current.forEach((handle) => window.clearTimeout(handle));
      timersRef.current.clear();
    };
  }, []);

  return { notifications, notify, dismiss };
}

export function NotificationCenter({ notifications, onDismiss }) {
  if (!notifications || notifications.length === 0) {
    return null;
  }

  return h(
    'div',
    { class: 'notification-center' },
    notifications.map((notification) =>
      h(
        'div',
        {
          key: notification.id,
          class: `notification notification--${notification.type ?? 'info'}`,
        },
        h('span', { class: 'notification__message' }, notification.message),
        h(
          'button',
          {
            type: 'button',
            class: 'notification__dismiss',
            onClick: () => onDismiss?.(notification.id),
            'aria-label': 'Dismiss notification',
          },
          '×',
        ),
      ),
    ),
  );
}
