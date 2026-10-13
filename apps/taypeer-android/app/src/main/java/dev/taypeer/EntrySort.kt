package dev.taypeer

import dev.taypeer.bridge.EntryRow

/** Local ordering of already masked rows; Rust remains the authority for matching. */
internal enum class EntrySort {
    TITLE_ASC, TITLE_DESC, USERNAME_ASC, MODIFIED_NEWEST;

    fun <T> comparator(database: (T) -> String, entry: (T) -> EntryRow): Comparator<T> = Comparator { left, right ->
        val first = entry(left)
        val second = entry(right)
        val primary = when (this) {
            TITLE_ASC -> first.title.compareTo(second.title, ignoreCase = true)
            TITLE_DESC -> second.title.compareTo(first.title, ignoreCase = true)
            USERNAME_ASC -> when {
                first.username.isNullOrEmpty() && !second.username.isNullOrEmpty() -> 1
                !first.username.isNullOrEmpty() && second.username.isNullOrEmpty() -> -1
                else -> first.username.orEmpty().compareTo(second.username.orEmpty(), ignoreCase = true)
            }
            MODIFIED_NEWEST -> second.modifiedAt.compareTo(first.modifiedAt)
        }
        if (primary != 0) primary else {
            val source = database(left).compareTo(database(right))
            if (source != 0) source else first.id.compareTo(second.id)
        }
    }
}
