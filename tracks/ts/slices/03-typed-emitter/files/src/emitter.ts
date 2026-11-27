/** A typed pub/sub bus. M maps each event name to its payload type. */
export class Emitter<M extends Record<string, unknown>> {
  /** Subscribe to name; returns a function that unsubscribes. */
  on<K extends keyof M>(name: K, fn: (payload: M[K]) => void): () => void {
    // TODO
    return () => {};
  }

  /** Deliver payload to every current listener of name, in subscription order. */
  emit<K extends keyof M>(name: K, payload: M[K]): void {
    // TODO
  }
}
