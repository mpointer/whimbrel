package com.mpointer.whimbrel.mobile.read

import com.mpointer.whimbrel.mobile.sync.SyncModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * Routine runs from every device, newest first; read-only. Ported from `RoutinesModel` in `RoutinesView.swift`.
 * Call from the main thread.
 */
class RoutinesModel {
    companion object {
        const val limit = 100
    }

    private val _runs = MutableStateFlow<List<PhoneRoutineRun>>(emptyList())
    val runs: StateFlow<List<PhoneRoutineRun>> = _runs.asStateFlow()
    private val _loaded = MutableStateFlow(false)
    val loaded: StateFlow<Boolean> = _loaded.asStateFlow()

    suspend fun reload(sync: SyncModel) {
        _runs.value = sync.read { it.routineRuns(limit) }.orEmpty().sortedByDescending { it.ranAtUnixMs }
        _loaded.value = true
    }
}
