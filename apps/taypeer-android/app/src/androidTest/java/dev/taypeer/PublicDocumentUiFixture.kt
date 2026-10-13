package dev.taypeer

import android.content.res.Configuration
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.AndroidComposeTestRule
import dev.taypeer.bridge.EntryTextField
import dev.taypeer.bridge.FormKind
import java.io.File
import java.util.Locale
import java.util.UUID

/** PUBLIC preparation uses real document commands; tested actions remain ordinary UI touches. */
internal class PublicDocumentUiFixture(private val compose: AndroidComposeTestRule<*, MainActivity>) {
    private val password = "PUBLIC lifecycle UI master password"
    private var database = ""
    private var originalTheme: ThemeMode? = null
    private var originalLanguage: UiLanguage? = null
    val state get() = compose.activity.documentState

    fun create() {
        compose.runOnIdle {
            val preferences = (compose.activity.application as TaypeerApplication).uiPreferences
            originalTheme = preferences.theme
            originalLanguage = preferences.language
            preferences.selectTheme(ThemeMode.SYSTEM)
            preferences.selectLanguage(UiLanguage.ENGLISH)
            state.create("PUBLIC lifecycle UI ${UUID.randomUUID().toString().take(8)}", null, password, newOperation())
        }
        await { state.overview != null && !state.opening && !state.navigating && state.catalog.any { it.database == state.overview?.database } }
        compose.runOnIdle { database = state.overview!!.database }
        waitFor("group-menu")
    }

    fun restorePreferences() {
        compose.runOnIdle {
            val preferences = (compose.activity.application as TaypeerApplication).uiPreferences
            originalTheme?.let(preferences::selectTheme)
            originalLanguage?.let(preferences::selectLanguage)
        }
    }

    fun group(name: String): String {
        compose.runOnIdle { state.beginMetadata(FormKind.NEW_GROUP, parent = null) }
        await { state.metadata != null && !state.navigating }
        compose.runOnIdle { state.editMetadata(name = name); state.saveNow() }
        await { state.activeForm == null && state.metadata == null && !state.navigating && state.overview?.groups?.any { it.name == name } == true }
        var id = ""
        compose.runOnIdle { id = state.overview!!.groups.single { it.name == name }.id }
        return id
    }

    fun entry(group: String, fields: PublicEntryFields): String {
        compose.runOnIdle { state.chooseGroup(group) }
        await { state.selectedGroup == group && !state.navigating }
        compose.runOnIdle { state.beginEntry() }
        await { state.entry != null && !state.navigating }
        saveFields(fields)
        var id = ""
        compose.runOnIdle { id = state.inspectedEntry!!.id }
        return id
    }

    fun update(entry: String, fields: PublicEntryFields) {
        compose.runOnIdle { state.beginEntry(entry) }
        await { state.entry?.source?.entry == entry && !state.navigating }
        saveFields(fields)
    }

    private fun saveFields(fields: PublicEntryFields) {
        compose.runOnIdle {
            state.edit(EntryTextField.TITLE, fields.title)
            state.edit(EntryTextField.USERNAME, fields.username)
            state.edit(EntryTextField.PASSWORD, fields.password)
            state.edit(EntryTextField.URL, fields.url)
            state.edit(EntryTextField.NOTES, fields.notes)
            state.editTags(fields.tags.joinToString("\n"))
            state.saveNow()
        }
        await { state.entry == null && state.activeForm == null && state.inspectedEntry?.title == fields.title && !state.navigating }
        waitFor("edit-entry")
    }

    fun savedBytes(): ByteArray {
        var path = ""
        compose.runOnIdle { path = state.catalog.single { it.database == database }.path }
        return File(path).readBytes()
    }

    fun showGroup(group: String) {
        if (compose.onAllNodes(hasTestTag("group-menu")).fetchSemanticsNodes().isEmpty()) {
            compose.onNodeWithTag("navigation-back").performClick()
            waitFor("group-menu")
        }
        compose.onNodeWithTag("group-menu").performClick()
        compose.onNodeWithTag("select-group-$group").performScrollTo().performClick()
        // A selected group can already equal the requested destination. The
        // drawer must finish closing before a physical tap reaches its row.
        waitFor("database-drawer-closed")
        await { state.selectedGroup == group && !state.navigating }
        waitFor("entry-search")
    }

    fun inspect(group: String, entry: String) {
        showGroup(group)
        compose.onNodeWithTag("entry-$entry").performScrollTo().performClick()
        await { state.inspectedEntry?.id == entry && state.properties?.entry == entry && !state.navigating }
        waitFor("edit-entry")
    }

    fun menu(control: String) {
        compose.onNodeWithTag("screen-actions").performClick()
        // The keyboard can constrain the popup while editing. Reach the real
        // scrollable menu item before sending a physical click at its center.
        compose.onNodeWithTag(control).performScrollTo().assertIsDisplayed().performClick()
    }

    fun reopen() {
        if (compose.onAllNodes(hasTestTag("group-menu")).fetchSemanticsNodes().isEmpty()) {
            compose.onNodeWithTag("navigation-back").performClick()
            waitFor("group-menu")
        }
        menu("lock-database")
        waitFor("master-password")
        compose.onNodeWithTag("master-password").performTextReplacement(password)
        compose.tapWorkingCopyUnlock(database)
        waitFor("group-menu")
        await { state.overview?.database == database && !state.opening && !state.navigating }
    }

    fun text(resource: Int, vararg arguments: Any): String {
        val configuration = Configuration(compose.activity.resources.configuration).apply { setLocale(Locale.ENGLISH) }
        return compose.activity.createConfigurationContext(configuration).getString(resource, *arguments)
    }

    fun await(predicate: () -> Boolean) = compose.waitUntil(20_000, predicate)
    fun waitAbsent(tag: String) = await { compose.onAllNodes(hasTestTag(tag)).fetchSemanticsNodes().isEmpty() }
    fun waitFor(tag: String) {
        try { await { compose.onAllNodes(hasTestTag(tag)).fetchSemanticsNodes().isNotEmpty() } }
        catch (timeout: ComposeTimeoutException) {
            var flags = ""
            compose.runOnIdle { flags = "overview=${state.overview != null}, entry=${state.entry != null}, metadata=${state.metadata != null}, inspected=${state.inspectedEntry != null}, form=${state.activeForm != null}, opening=${state.opening}, navigating=${state.navigating}, decision=${state.navigationDecision}, invalid=${state.invalidInput}, failure=${state.error}" }
            throw AssertionError("Missing control $tag; $flags", timeout)
        }
    }
}

/** Entirely synthetic values; they must never be replaced by a user's document or credentials. */
internal data class PublicEntryFields(
    val title: String = "PUBLIC lifecycle current entry",
    val username: String = "PUBLIC current username",
    val password: String = "PUBLIC lifecycle current password",
    val url: String = "https://example.invalid/current",
    val notes: String = "PUBLIC current notes",
    val tags: List<String> = listOf("PUBLIC lifecycle tag", "PUBLIC current tag"),
)
