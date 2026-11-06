package com.mpointer.whimbrel.mobile.sync

import android.content.SharedPreferences
import java.io.FileNotFoundException
import java.nio.file.NoSuchFileException

/**
 * A marker that the local store must be deleted, set before the wipe starts and cleared only once the
 * directory is gone, so a wipe cut short (process killed, delete failed) finishes at the next launch.
 */
object StoreRemoval {
    const val pendingKey = "storeRemovalPending"

    /** `commit`, not `apply`: the marker must be on disk before the wipe begins. */
    fun markPending(prefs: SharedPreferences) {
        prefs.edit().putBoolean(pendingKey, true).commit()
    }

    /** Runs [remove]; clears the marker on success or when the store is already gone. Any other failure keeps it. */
    fun finish(prefs: SharedPreferences, remove: () -> Unit) {
        try {
            remove()
        } catch (_: FileNotFoundException) {
        } catch (_: NoSuchFileException) {
        } catch (_: Exception) {
            return
        }
        prefs.edit().remove(pendingKey).commit()
    }

    /** At launch: finishes a removal a previous run left pending. */
    fun finishIfPending(prefs: SharedPreferences, remove: () -> Unit) {
        if (prefs.getBoolean(pendingKey, false)) finish(prefs, remove)
    }
}
