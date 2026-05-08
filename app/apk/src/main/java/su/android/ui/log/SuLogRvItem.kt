package su.android.ui.log

import androidx.databinding.Bindable
import su.android.BR
import su.android.R
import su.android.core.AppContext
import su.android.core.ktx.timeDateFormat
import su.android.core.ktx.toTime
import su.android.core.model.su.SuLog
import su.android.databinding.DiffItem
import su.android.databinding.ObservableRvItem
import su.android.databinding.set
import su.android.core.R as CoreR

class SuLogRvItem(val log: SuLog) : ObservableRvItem(), DiffItem<SuLogRvItem> {

    override val layoutRes = R.layout.item_log_access_md2

    val info = genInfo()

    @get:Bindable
    var isTop = false
        set(value) = set(value, field, { field = it }, BR.top)

    @get:Bindable
    var isBottom = false
        set(value) = set(value, field, { field = it }, BR.bottom)

    override fun itemSameAs(other: SuLogRvItem) = log.appName == other.log.appName

    private fun genInfo(): String {
        val res = AppContext.resources
        val sb = StringBuilder()
        val date = log.time.toTime(timeDateFormat)
        val toUid = res.getString(CoreR.string.target_uid, log.toUid)
        val fromPid = res.getString(CoreR.string.pid, log.fromPid)
        sb.append("$date\n$toUid  $fromPid")
        if (log.target != -1) {
            val pid = if (log.target == 0) "magiskd" else log.target.toString()
            val target = res.getString(CoreR.string.target_pid, pid)
            sb.append("  $target")
        }
        if (log.context.isNotEmpty()) {
            val context = res.getString(CoreR.string.selinux_context, log.context)
            sb.append("\n$context")
        }
        if (log.gids.isNotEmpty()) {
            val gids = res.getString(CoreR.string.supp_group, log.gids)
            sb.append("\n$gids")
        }
        sb.append("\n${log.command}")
        return sb.toString()
    }
}
