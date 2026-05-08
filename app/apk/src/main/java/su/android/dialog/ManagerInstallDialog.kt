package su.android.dialog

import su.android.core.AppContext
import su.android.core.Info
import su.android.core.R
import su.android.core.download.DownloadEngine
import su.android.core.download.Subject
import su.android.view.MagiskDialog
import java.io.File

class ManagerInstallDialog : MarkDownDialog() {

    override suspend fun getMarkdownText(): String {
        val text = Info.update.note
        // Cache the changelog
        File(AppContext.cacheDir, "${Info.update.versionCode}.md").writeText(text)
        return text
    }

    override fun build(dialog: MagiskDialog) {
        super.build(dialog)
        dialog.apply {
            setCancelable(true)
            setButton(MagiskDialog.ButtonType.POSITIVE) {
                text = R.string.install
                onClick { DownloadEngine.startWithActivity(activity, Subject.App()) }
            }
            setButton(MagiskDialog.ButtonType.NEGATIVE) {
                text = android.R.string.cancel
            }
        }
    }

}
