// One toast rendering for both windows.

/** Renders nothing when there is no message. */
export function Toast({ message }: { message: string }) {
  if (!message) return null;
  return (
    <div className="toast" role="status" aria-live="polite">
      {message}
    </div>
  );
}
