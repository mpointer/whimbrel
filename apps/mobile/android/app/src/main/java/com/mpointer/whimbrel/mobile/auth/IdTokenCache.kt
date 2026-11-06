package com.mpointer.whimbrel.mobile.auth

import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/**
 * Caches a Google ID token until a minute before it expires. The pending-approval poll asks every
 * 3-30 s, so this is what keeps it from hitting Credential Manager each time. Concurrent callers wait
 * on one fetch; a failed fetch caches nothing.
 */
class IdTokenCache(
    private val now: () -> Long,
    private val fetch: suspend () -> String,
    private val expSeconds: (String) -> Long?,
) {
    private val lock = Mutex()
    @Volatile private var token: String? = null
    @Volatile private var refreshAtMs = 0L
    // Bumped by clear(); a fetch that started before it must not store its token.
    @Volatile private var generation = 0

    suspend fun get(): String = lock.withLock {
        token?.takeIf { now() < refreshAtMs } ?: run {
            val started = generation
            fetch().also {
                if (started == generation) {
                    token = it
                    // An unreadable exp is never cached (refreshAtMs in the past), so it refetches next time.
                    refreshAtMs = expSeconds(it)?.let { exp -> exp * 1000 - 60_000 } ?: 0L
                }
            }
        }
    }

    /** Stores a token got interactively, so the next [get] doesn't fetch again. */
    suspend fun seed(token: String) = lock.withLock {
        generation++
        this.token = token
        refreshAtMs = expSeconds(token)?.let { exp -> exp * 1000 - 60_000 } ?: 0L
    }

    fun clear() {
        generation++
        token = null
        refreshAtMs = 0L
    }
}
