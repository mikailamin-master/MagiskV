package su.android.ui.install

import su.android.R
import su.android.arch.BaseFragment
import su.android.arch.viewModel
import su.android.databinding.FragmentInstallMd2Binding
import su.android.core.R as CoreR

class InstallFragment : BaseFragment<FragmentInstallMd2Binding>() {

    override val layoutRes = R.layout.fragment_install_md2
    override val viewModel by viewModel<InstallViewModel>()

    override fun onStart() {
        super.onStart()
        requireActivity().setTitle(CoreR.string.install)
    }
}
