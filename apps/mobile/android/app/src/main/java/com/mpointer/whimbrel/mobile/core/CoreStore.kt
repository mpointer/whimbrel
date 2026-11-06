package com.mpointer.whimbrel.mobile.core

import java.io.File

/** An open memory store. Not thread-safe; callers serialize. */
class CoreStore private constructor(val handle: Long, private val native: CoreNative) : AutoCloseable {
    private var closed = false

    companion object {
        fun open(path: File, native: CoreNative = Native): CoreStore? =
            native.memoryOpen(path.path).takeIf { it != 0L }?.let { CoreStore(it, native) }
    }

    override fun close() {
        if (closed) return
        closed = true
        native.memoryClose(handle)
    }
}
