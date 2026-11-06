package com.mpointer.whimbrel.mobile.auth

import androidx.annotation.StringRes
import com.mpointer.whimbrel.mobile.R
import java.net.URI

/** Whether this build carries a usable Web client id and Worker URL; mirrors the iPhone's rule. */
object EnrollmentConfig {
    private const val SUFFIX = ".apps.googleusercontent.com"

    fun isUsableClientId(id: String) = id.endsWith(SUFFIX) && id.length > SUFFIX.length

    fun isUsableWorkerUrl(url: String) =
        runCatching { URI(url) }.getOrNull()?.let { it.scheme == "https" && !it.host.isNullOrEmpty() } ?: false

    @StringRes
    fun configErrorRes(webClientId: String, workerUrl: String): Int? {
        val idOk = isUsableClientId(webClientId)
        val urlOk = isUsableWorkerUrl(workerUrl)
        return when {
            idOk && urlOk -> null
            urlOk -> R.string.config_missing_client_id
            idOk -> R.string.config_missing_worker_url
            else -> R.string.config_missing_both
        }
    }
}
