package com.mpointer.whimbrel.mobile.ui

import androidx.annotation.StringRes
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.mpointer.whimbrel.mobile.R
import com.mpointer.whimbrel.mobile.core.CoreUserMessage
import com.mpointer.whimbrel.mobile.core.LimitHolder
import com.mpointer.whimbrel.mobile.sync.ActionError
import com.mpointer.whimbrel.mobile.sync.EnrollmentModel
import com.mpointer.whimbrel.mobile.sync.EnrollmentStatus
import com.mpointer.whimbrel.mobile.sync.LimitMessage
import kotlinx.coroutines.launch

/** Resolves a string resource id and its format args; the screen passes `context.getString`, tests the real strings.xml. */
typealias StringResolver = (Int, Array<Any>) -> String

/** Text that is only known at run time. It carries values, not English, and is resolved in the screen. */
sealed interface EnrollmentText {
    fun localized(resolve: StringResolver): String

    /** Already a string: the build's configuration error, made from a resource by the Application. */
    data class Literal(val text: String) : EnrollmentText {
        override fun localized(resolve: StringResolver) = text
    }

    data class Res(@StringRes val id: Int, val args: List<String> = emptyList()) : EnrollmentText {
        override fun localized(resolve: StringResolver) = resolve(id, args.toTypedArray())
    }

    /** The core's message for a failure; null (no message from the core) is a generic error. */
    data class Core(val message: CoreUserMessage?) : EnrollmentText {
        override fun localized(resolve: StringResolver) =
            message?.localized(resolve) ?: resolve(R.string.enroll_error_generic, emptyArray())
    }

    data class Limit(val message: LimitMessage) : EnrollmentText {
        override fun localized(resolve: StringResolver) = message.localized(resolve)
    }
}

sealed interface EnrollmentAction {
    data object SignIn : EnrollmentAction
    data object TryAgain : EnrollmentAction
    data class UseThisPhoneInstead(val holder: LimitHolder) : EnrollmentAction
    data object JoinWaitlist : EnrollmentAction
}

/**
 * What the enrollment screen shows for a status. Title and body are either a fixed resource ([titleRes],
 * [bodyRes]) or run-time text ([title], [body]); a status sets one of the two. [code] is shown as is, and
 * [codeDescription] is what a screen reader says instead.
 */
data class EnrollmentContent(
    @StringRes val titleRes: Int? = null,
    val title: EnrollmentText? = null,
    @StringRes val bodyRes: Int? = null,
    val body: EnrollmentText? = null,
    val code: String? = null,
    val codeDescription: EnrollmentText? = null,
    val actions: List<EnrollmentAction> = emptyList(),
    val actionsEnabled: Boolean = true,
    val showsProgress: Boolean = false,
)

/** Copy and layout from the iPhone's `EnrollmentView.swift`, with "iPhone" as "phone". Pure, so it is unit tested. */
fun enrollmentContent(status: EnrollmentStatus, swapRequested: Boolean): EnrollmentContent = when (status) {
    is EnrollmentStatus.NotConfigured -> EnrollmentContent(
        body = EnrollmentText.Literal(status.message),
        actions = listOf(EnrollmentAction.SignIn),
        actionsEnabled = false,
    )
    EnrollmentStatus.SignedOut -> EnrollmentContent(bodyRes = R.string.enroll_signed_out, actions = listOf(EnrollmentAction.SignIn))
    EnrollmentStatus.SigningIn -> EnrollmentContent(bodyRes = R.string.enroll_signing_in, showsProgress = true)
    is EnrollmentStatus.Pending -> codeContent(
        R.string.enroll_waiting_title,
        if (swapRequested) R.string.enroll_pending_swap else R.string.enroll_pending_detail,
        status.code,
    )
    is EnrollmentStatus.WaitingForGroupKey -> codeContent(R.string.enroll_reapprove_title, R.string.enroll_reapprove_detail, status.code)
    EnrollmentStatus.NeedsDesktopFirst -> EnrollmentContent(bodyRes = R.string.enroll_needs_desktop, actions = listOf(EnrollmentAction.TryAgain))
    is EnrollmentStatus.DeviceLimit -> EnrollmentContent(
        title = EnrollmentText.Limit(EnrollmentModel.limitMessage(status.holders)),
        actions = status.holders.map { EnrollmentAction.UseThisPhoneInstead(it) } + EnrollmentAction.JoinWaitlist,
    )
    EnrollmentStatus.Approved -> EnrollmentContent() // MainActivity shows the signed-in app instead
    EnrollmentStatus.Removed -> EnrollmentContent(bodyRes = R.string.enroll_removed, actions = listOf(EnrollmentAction.SignIn))
    is EnrollmentStatus.Error -> EnrollmentContent(body = EnrollmentText.Core(status.message), actions = listOf(EnrollmentAction.SignIn))
}

private fun codeContent(@StringRes titleRes: Int, @StringRes bodyRes: Int, code: String) = EnrollmentContent(
    titleRes = titleRes,
    bodyRes = bodyRes,
    code = code,
    // Spoken group by group: "K7 · 4Q" would read the dots out.
    codeDescription = EnrollmentText.Res(R.string.enroll_code_description, listOf(code.replace(" · ", " "))),
    showsProgress = true,
)

@Composable
fun EnrollmentScreen(model: EnrollmentModel) {
    val status by model.status.collectAsState()
    val swapRequested by model.swapRequested.collectAsState()
    val actionError by model.actionError.collectAsState()
    val content = remember(status, swapRequested) { enrollmentContent(status, swapRequested) }
    val context = LocalContext.current
    val resolve: StringResolver = { id, args -> context.getString(id, *args) }
    val scope = rememberCoroutineScope() // Main: the model is not thread-safe

    Surface(Modifier.fillMaxSize()) {
        Column(
            Modifier.safeDrawingPadding().verticalScroll(rememberScrollState()).padding(24.dp).fillMaxWidth(),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Column(
                Modifier.widthIn(max = 480.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(20.dp),
            ) {
                Text(stringResource(R.string.app_name), style = MaterialTheme.typography.headlineLarge, fontWeight = FontWeight.Bold)
                val title = content.titleRes?.let { stringResource(it) } ?: content.title?.localized(resolve)
                title?.let { Text(it, style = MaterialTheme.typography.titleLarge, textAlign = TextAlign.Center) }
                content.code?.let { CodeText(it, content.codeDescription?.localized(resolve).orEmpty()) }
                val body = content.bodyRes?.let { stringResource(it) } ?: content.body?.localized(resolve)
                body?.let { Text(it, textAlign = TextAlign.Center, color = MaterialTheme.colorScheme.onSurfaceVariant) }
                if (content.showsProgress) CircularProgressIndicator()
                for (action in content.actions) {
                    when (action) {
                        EnrollmentAction.SignIn ->
                            Button({ scope.launch { model.signIn() } }, enabled = content.actionsEnabled) { Text(stringResource(R.string.enroll_sign_in)) }
                        EnrollmentAction.TryAgain ->
                            Button({ scope.launch { model.signIn() } }) { Text(stringResource(R.string.enroll_try_again)) }
                        is EnrollmentAction.UseThisPhoneInstead -> UseThisPhoneButton(model, action.holder)
                        EnrollmentAction.JoinWaitlist -> Waitlist(model)
                    }
                }
                actionError?.let {
                    Text(it.localized(resolve), style = MaterialTheme.typography.bodySmall, textAlign = TextAlign.Center, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
        }
    }
}

/** The comparison code: monospaced, one line, shrinking until it fits (a Fold's cover screen is narrow). */
@Composable
private fun CodeText(code: String, description: String) {
    var size by remember(code) { mutableStateOf(34.sp) }
    Text(
        code,
        Modifier.semantics { contentDescription = description },
        color = MaterialTheme.colorScheme.primary,
        fontFamily = FontFamily.Monospace,
        fontWeight = FontWeight.SemiBold,
        fontSize = size,
        maxLines = 1,
        softWrap = false,
        onTextLayout = { if (it.didOverflowWidth && size > 17.sp) size *= 0.9f },
    )
}

@Composable
private fun UseThisPhoneButton(model: EnrollmentModel, holder: LimitHolder) {
    val scope = rememberCoroutineScope()
    var working by remember { mutableStateOf(false) }
    Button({ scope.launch { working = true; model.swap(holder); working = false } }, enabled = !working) {
        Text(stringResource(R.string.enroll_use_this_phone))
    }
}

@Composable
private fun Waitlist(model: EnrollmentModel) {
    val scope = rememberCoroutineScope()
    var wants by remember { mutableStateOf(emptyList<String>()) }
    var joined by remember { mutableStateOf(false) }
    var working by remember { mutableStateOf(false) }
    HorizontalDivider()
    if (joined) {
        Text(stringResource(R.string.enroll_waitlist_joined), textAlign = TextAlign.Center, color = MaterialTheme.colorScheme.onSurfaceVariant)
        return
    }
    Text(stringResource(R.string.enroll_waitlist_prompt), color = MaterialTheme.colorScheme.onSurfaceVariant)
    for ((labelRes, want) in EnrollmentModel.waitlistOptions) {
        val checked = want in wants
        Row(
            Modifier.fillMaxWidth().toggleable(checked, role = Role.Checkbox) { wants = if (it) wants + want else wants - want },
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Checkbox(checked, onCheckedChange = null)
            Text(stringResource(labelRes), Modifier.padding(start = 12.dp, top = 8.dp, bottom = 8.dp))
        }
    }
    OutlinedButton(
        { scope.launch { working = true; joined = model.joinWaitlist(wants.sorted()); working = false } },
        enabled = wants.isNotEmpty() && !working,
    ) { Text(stringResource(R.string.enroll_join_waitlist)) }
}
