package io.github.ltdjorge.pzmap;

import java.lang.ref.Cleaner;
import java.lang.ref.Reference;
import java.util.concurrent.locks.ReentrantReadWriteLock;
import java.util.function.LongConsumer;
import java.util.function.LongFunction;

/**
 * Owns a pointer to a value allocated by the native library.
 *
 * <p>{@link #close()} frees the value. If the owner is garbage collected without being closed, a
 * {@link Cleaner} frees it instead. Callers should close explicitly, because the garbage collector
 * may run the cleaner much later or not at all.
 *
 * <p>Every native call goes through {@link #call}, which holds a read lock during the call. So calls
 * from several threads can run at the same time. {@link #close()} takes the write lock, so it waits
 * until running calls finish before it frees the value. A call after close throws
 * {@link IllegalStateException}.
 */
final class NativeHandle implements AutoCloseable {

    /** One cleaner thread shared by every handle in the library. */
    private static final Cleaner CLEANER = Cleaner.create();

    private final State state;
    private final String what;
    private final Cleaner.Cleanable cleanable;

    /**
     * The pointer and its lock, shared with the cleaner.
     *
     * <p>This class must not refer to its {@link NativeHandle}. If it did, the handle would never
     * become unreachable and the cleaner would never run.
     */
    private static final class State implements Runnable {
        final ReentrantReadWriteLock lock = new ReentrantReadWriteLock();
        final LongConsumer free;
        long pointer;

        State(long pointer, LongConsumer free) {
            this.pointer = pointer;
            this.free = free;
        }

        @Override
        public void run() {
            lock.writeLock().lock();
            try {
                if (pointer != 0) {
                    free.accept(pointer);
                    pointer = 0;
                }
            } finally {
                lock.writeLock().unlock();
            }
        }
    }

    /**
     * Takes ownership of a pointer returned by native code.
     *
     * @param pointer the value returned by a native {@code nativeParse*} method, which must not be
     *     zero
     * @param what the class name to use in the exception thrown after close
     * @param free the native method that frees this kind of value
     * @throws IllegalStateException if {@code pointer} is zero
     */
    NativeHandle(long pointer, String what, LongConsumer free) {
        if (pointer == 0) {
            throw new IllegalStateException("native " + what + " parse returned no handle");
        }
        this.state = new State(pointer, free);
        this.what = what;
        this.cleanable = CLEANER.register(this, state);
    }

    /**
     * Runs a native call with the pointer, which stays valid until the call returns.
     *
     * @param nativeCall the native call, which receives the pointer
     * @return what {@code nativeCall} returned
     * @throws IllegalStateException if the handle has been closed
     */
    <R> R call(LongFunction<R> nativeCall) {
        state.lock.readLock().lock();
        try {
            if (state.pointer == 0) {
                throw new IllegalStateException(what + " has been closed");
            }
            return nativeCall.apply(state.pointer);
        } finally {
            state.lock.readLock().unlock();
            // Keeps this handle reachable until here, so the cleaner cannot free the pointer while
            // the native call is still using it.
            Reference.reachabilityFence(this);
        }
    }

    /** Returns whether the native value has been freed. */
    boolean isClosed() {
        state.lock.readLock().lock();
        try {
            return state.pointer == 0;
        } finally {
            state.lock.readLock().unlock();
        }
    }

    /**
     * Frees the native value once running calls have finished.
     *
     * <p>Calling this again does nothing.
     */
    @Override
    public void close() {
        cleanable.clean();
    }
}
