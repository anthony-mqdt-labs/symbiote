/** A typed pub/sub bus. M maps each event name to its payload type. */
export class Emitter<M extends Record<string, unknown>> {
  private listeners: { [K in keyof M]?: Set<(payload: M[K]) => void> } = {};

  /** Subscribe to name; returns a function that unsubscribes. */
  on<K extends keyof M>(name: K, fn: (payload: M[K]) => void): () => void {
    const set = (this.listeners[name] ??= new Set());
    set.add(fn);
    return () => {
      set.delete(fn);
    };
  }

  /** Deliver payload to every current listener of name, in subscription order. */
  emit<K extends keyof M>(name: K, payload: M[K]): void {
    for (const fn of [...(this.listeners[name] ?? [])]) fn(payload);
  }
}
