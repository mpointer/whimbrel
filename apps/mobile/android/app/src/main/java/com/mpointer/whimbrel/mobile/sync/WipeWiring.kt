package com.mpointer.whimbrel.mobile.sync

import android.content.SharedPreferences
import android.util.Log
import java.io.File
import java.io.IOException

/** [PrefStore] over SharedPreferences. `commit` throughout: a wipe must be on disk before it reports done. */
class SharedPrefStore(private val sp: SharedPreferences) : PrefStore {
    override fun getString(key: String): String? = sp.getString(key, null)
    override fun getLong(key: String): Long? = if (sp.contains(key)) sp.getLong(key, 0) else null
    override fun getBoolean(key: String): Boolean = sp.getBoolean(key, false)
    override fun putString(key: String, v: String) { sp.edit().putString(key, v).commit() }
    override fun putLong(key: String, v: Long) { sp.edit().putLong(key, v).commit() }
    override fun putBoolean(key: String, v: Boolean) { sp.edit().putBoolean(key, v).commit() }
    override fun clearPrefix(prefix: String) {
        val e = sp.edit()
        sp.all.keys.filter { it.startsWith(prefix) }.forEach(e::remove)
        e.commit()
    }
}

/** `deleteRecursively` reports failure by returning false; a missing directory is already done. */
fun deleteStoreDir(dir: File) {
    if (!dir.deleteRecursively() && dir.exists()) throw IOException("store directory not deleted")
}

/**
 * What the enrollment model runs on revoke: forget synced prefs and keys, mark the store removal pending,
 * then have [sync] close the store and delete [storeDir] on the store thread. [removal] is the same
 * SharedPreferences [prefs] wraps (the marker lives there).
 */
fun wipeHandler(
    prefs: PrefStore,
    removal: SharedPreferences,
    keys: PhoneKeyStore,
    sync: SyncModel,
    storeDir: File,
    log: (String) -> Unit = { Log.w("wipe", it) },
): () -> Unit = {
    // A failed local wipe must not keep the store alive: mark and reset regardless.
    runCatching { PhoneSettingsSync.wipeLocal(prefs, keys) }.onFailure { log("local wipe failed: ${it.javaClass.simpleName}") }
    StoreRemoval.markPending(removal)
    sync.reset { StoreRemoval.finish(removal) { deleteStoreDir(storeDir) } }
}

/**
 * The store opener, on the store thread: a removal left pending (a failed delete) is finished first, and
 * while it still cannot finish the old directory is never reopened (the caller reports storage unavailable).
 */
fun <T> openStoreGuarded(removal: SharedPreferences, storeDir: File, open: () -> T?): T? {
    StoreRemoval.finishIfPending(removal) { deleteStoreDir(storeDir) }
    return if (removal.getBoolean(StoreRemoval.pendingKey, false)) null else open()
}
