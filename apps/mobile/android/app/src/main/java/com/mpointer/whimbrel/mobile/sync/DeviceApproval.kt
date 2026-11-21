package com.mpointer.whimbrel.mobile.sync

import com.mpointer.whimbrel.mobile.BuildConfig

/**
 * Whether this build may approve a waiting device. Approving wraps and hands over the account key (PR #914),
 * and that has never been tried as a real two-device approval, so release builds have it OFF
 * (see DEVICE_APPROVAL_ENABLED in app/build.gradle.kts) and debug builds have it on for that test.
 * Mike removes this gate, and the flag, after a real two-device test.
 *
 * Both the Devices screen (no Approve button) and [EnrollmentModel.approve] (refuses before touching any
 * key) consult it.
 */
object DeviceApproval {
    val enabled: Boolean get() = isEnabled(BuildConfig.DEVICE_APPROVAL_ENABLED)

    /** The pure rule, so a test can drive both flag values. */
    fun isEnabled(buildFlag: Boolean): Boolean = buildFlag
}
