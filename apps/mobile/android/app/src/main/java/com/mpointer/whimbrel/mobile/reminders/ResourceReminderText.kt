package com.mpointer.whimbrel.mobile.reminders

import android.content.Context
import com.mpointer.whimbrel.mobile.R

/** The reminder copy from strings.xml. */
class ResourceReminderText(private val context: Context) : ReminderText {
    override fun meetingIn(title: String, minutes: Int) = context.getString(R.string.reminder_meeting_in, title, minutes)
    override fun withTopic(line: String, topic: String) = context.getString(R.string.reminder_with_topic, line, topic)
    override fun stepsDue(count: Int) = context.resources.getQuantityString(R.plurals.reminder_steps_due, count, count)
}
