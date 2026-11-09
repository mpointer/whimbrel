package com.mpointer.whimbrel.mobile.sync

import com.mpointer.whimbrel.mobile.R
import java.io.File

/** Resolves string ids against the real strings.xml, so a test fails if an entry is missing or reworded. */
internal object StringCatalog {
    private val catalog: Map<String, String> =
        Regex("""<string name="(\w+)">(.*)</string>""")
            .findAll(File("src/main/res/values/strings.xml").readText())
            .associate { it.groupValues[1] to it.groupValues[2].replace("\\'", "'").replace("&amp;", "&") }

    val resolve: (Int, Array<Any>) -> String = { id, args ->
        val name = R.string::class.java.fields.first { it.getInt(null) == id }.name
        String.format(catalog.getValue(name), *args)
    }
}
