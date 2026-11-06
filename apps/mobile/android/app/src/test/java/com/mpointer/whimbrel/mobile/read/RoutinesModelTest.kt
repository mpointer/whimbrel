package com.mpointer.whimbrel.mobile.read

import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class RoutinesModelTest {
    private fun run(id: Long, at: Long, status: String = "success") = PhoneRoutineRun(id, "R$id", at, status, "out")

    @Test
    fun runsLoadNewestFirstAndAreEmptyWithNoStore() = runTest {
        val store = ScriptedStore().apply { runs = listOf(run(1, 100), run(2, 300), run(3, 200)) }
        val model = RoutinesModel()
        assertFalse(model.loaded.value)
        model.reload(syncWith(store, backgroundScope))
        assertEquals(listOf(2L, 3L, 1L), model.runs.value.map { it.id })
        assertTrue(model.loaded.value)

        val none = RoutinesModel()
        none.reload(syncWith(null, backgroundScope))
        assertTrue(none.runs.value.isEmpty())
        assertTrue(none.loaded.value)
    }

    @Test
    fun runStatusFlags() {
        assertTrue(run(1, 1, "success").succeeded)
        assertFalse(run(2, 1, "failed").succeeded)
        assertFalse(run(3, 1, "").succeeded)
    }
}
