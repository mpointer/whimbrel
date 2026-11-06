package com.mpointer.whimbrel.mobile.core

/** UTF-8 bytes plus one trailing NUL, the shape the C core expects. */
fun String.toCString(): ByteArray = toByteArray(Charsets.UTF_8) + 0

/** Decodes UTF-8 bytes copied out of a core `char*`; null stays null. */
fun ByteArray?.fromCOutput(): String? = this?.toString(Charsets.UTF_8)
