import { useCallback, useEffect, useRef, useState } from "react";

const DEFAULT_DURATION_MS = 2200;

export interface ToastController {
  message: string;
  show: (message: string, durationMs?: number) => void;
  dismiss: () => void;
}

/** Transient message state. Cleans up its own timer on unmount. */
export function useToast(): ToastController {
  const [message, setMessage] = useState("");
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const show = useCallback((next: string, durationMs = DEFAULT_DURATION_MS) => {
    setMessage(next);
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => setMessage(""), durationMs);
  }, []);

  const dismiss = useCallback(() => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
    setMessage("");
  }, []);

  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current);
    },
    [],
  );

  return { message, show, dismiss };
}
