package su.android.view

import su.android.R
import su.android.databinding.DiffItem
import su.android.databinding.ItemWrapper
import su.android.databinding.RvItem

class TextItem(override val item: Int) : RvItem(), DiffItem<TextItem>, ItemWrapper<Int> {
    override val layoutRes = R.layout.item_text
}
