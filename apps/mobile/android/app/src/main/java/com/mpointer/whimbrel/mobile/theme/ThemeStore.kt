package com.mpointer.whimbrel.mobile.theme

import com.mpointer.whimbrel.mobile.sync.PrefStore
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

/** The installed themes and the selected one; the selection is per device. Works on every phone (no Native). */
class ThemeStore(private val prefs: PrefStore) {
    private var storedId = prefs.getString(SELECTED_KEY) ?: ThemeDefaults.theme.id
    private val _themes = MutableStateFlow(listOf(ThemeDefaults.theme))
    private val _selectedId = MutableStateFlow(resolve(storedId, _themes.value))

    val themes: StateFlow<List<Theme>> = _themes
    val selectedId: StateFlow<String> = _selectedId

    private fun resolve(id: String, themes: List<Theme>) = if (themes.any { it.id == id }) id else ThemeDefaults.theme.id

    /** Replaces the installed themes; Default is always first and always present. */
    fun install(themes: List<Theme>) {
        val all = listOf(ThemeDefaults.theme) + themes.filter { it.id != ThemeDefaults.theme.id }
        _themes.value = all
        _selectedId.value = resolve(storedId, all)
    }

    fun select(id: String) {
        if (_themes.value.none { it.id == id }) return
        storedId = id
        prefs.putString(SELECTED_KEY, id)
        _selectedId.value = id
    }

    companion object {
        const val SELECTED_KEY = "com.mpointer.kestrel.themeId"
    }
}
