package dev.taypeer

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.taypeer.bridge.EntryRow
import dev.taypeer.bridge.WorkingCopyView
import dev.taypeer.platform.OpenDocumentSource
import kotlinx.coroutines.*

internal enum class SearchScope { CURRENT, ALL_UNLOCKED }
internal data class SearchSourceKey(val database: String, val path: String, val generation: ULong)
internal data class SearchResultRow(val copy: WorkingCopyView, val generation: ULong,
    val databaseName: String, val groupName: String, val entry: EntryRow) {
    val sourceKey get() = SearchSourceKey(copy.database, copy.path, generation)
}
internal data class SearchReturn(val copy: WorkingCopyView, val generation: ULong,
    val group: String?, val query: String, val scope: SearchScope)
private data class SearchBatch(val source: SearchSourceKey, val name: String?,
    val rows: List<SearchResultRow>, val failed: Boolean)

/** Owns requests and routes; the existing Rust Entries command owns every matching rule. */
internal class SearchState(private val owner: CoroutineScope,
    private val sources: () -> List<OpenDocumentSource>, private val selected: () -> OpenDocumentSource?,
    private val selectedGroup: () -> String?, private val selectedName: () -> String?,
    private val order: () -> EntrySort = { EntrySort.TITLE_ASC }) {
    var query by mutableStateOf(""); private set
    var scope by mutableStateOf(SearchScope.CURRENT); private set
    var showing by mutableStateOf(false); private set
    var results by mutableStateOf<List<SearchResultRow>>(emptyList()); private set
    var busy by mutableStateOf(false); private set
    var error by mutableStateOf(false); private set
    var returning by mutableStateOf<SearchReturn?>(null); private set
    var previewEpoch by mutableStateOf(0L); private set
    private var origin: SearchReturn? = null
    private var sequence = 0L
    private var job: Job? = null
    private var observed = emptySet<SearchSourceKey>()
    private val names = mutableStateMapOf<SearchSourceKey, String>()
    private val applicationRevisions = mutableMapOf<SearchSourceKey, ULong>()
    val currentName: String? get() = if (origin != null) names[contextKey(origin!!)] else selectedName()

    init {
        owner.launch {
            while (isActive) {
                delay(500)
                if (!showing) continue
                val request = sequence
                val updates = querySources().map { source -> async(Dispatchers.IO) {
                    val revision = try { source.session.applicationState()?.appliedRevision }
                        catch (cancelled: CancellationException) { throw cancelled }
                        catch (_: Exception) { null }
                    key(source) to revision
                } }.awaitAll()
                if (request != sequence || !showing) continue
                var changed = false
                updates.forEach { (key, revision) ->
                    if (revision != null) {
                        val previous = applicationRevisions.put(key, revision)
                        if (previous != null && previous != revision) changed = true
                    }
                }
                if (changed) { previewEpoch++; refresh() }
            }
        }
    }
    private fun key(source: OpenDocumentSource) = SearchSourceKey(source.copy.database, source.copy.path, source.generation)
    private fun contextKey(context: SearchReturn) = SearchSourceKey(context.copy.database, context.copy.path, context.generation)
    fun source(row: SearchResultRow): OpenDocumentSource? = sources().firstOrNull { key(it) == row.sourceKey && it.permitted() }
    fun source(context: SearchReturn): OpenDocumentSource? = sources().firstOrNull { key(it) == contextKey(context) && it.permitted() }
    private fun captureOrigin() {
        if (origin != null) return
        selected()?.let { source ->
            origin = SearchReturn(source.copy, source.generation, selectedGroup(), query, scope)
            selectedName()?.let { names[key(source)] = it }
        }
    }
    private fun querySources(): List<OpenDocumentSource> {
        val available = sources().filter(OpenDocumentSource::permitted)
        return if (scope == SearchScope.ALL_UNLOCKED) available
        else available.filter { source -> origin?.let { key(source) == contextKey(it) } == true }
    }
    fun update(value: String) {
        if (value.isEmpty()) { close(); return }
        if (value != query) results = emptyList()
        query = value; captureOrigin(); showing = true; returning = null
        refresh()
    }
    fun selectScope(value: SearchScope) {
        if (value != scope) results = emptyList()
        scope = value; captureOrigin()
        showing = query.isNotEmpty() || value == SearchScope.ALL_UNLOCKED
        if (showing) refresh()
    }
    fun beginResult(row: SearchResultRow): Boolean {
        if (source(row) == null || results.none { it.sourceKey == row.sourceKey && it.entry.id == row.entry.id }) return false
        val base = origin ?: return false
        returning = base.copy(query = query, scope = scope); showing = false
        job?.cancel(); sequence++; busy = false
        return true
    }
    fun returnToResults() {
        val context = returning ?: return
        origin = context; query = context.query; scope = context.scope; returning = null; showing = true
        refresh()
    }
    fun refresh() {
        if (!showing) return
        job?.cancel()
        val request = ++sequence
        val requestedQuery = query
        val requestedScope = scope
        val requestedSources = querySources()
        observed = requestedSources.map(::key).toSet()
        // A refresh of the same projection keeps its rows stable while Rust
        // supplies the next view. Query/scope changes clear them at the action,
        // and every activation still rechecks the exact live source below.
        results = results.filter { row -> requestedSources.any { key(it) == row.sourceKey } }
        busy = true; error = false
        job = owner.launch {
            val views = requestedSources.map { source -> async(Dispatchers.IO) {
                try {
                    val view = source.session.overview(requestedQuery)
                    if (!source.permitted() || view.database != source.copy.database || view.generation != source.generation)
                        SearchBatch(key(source), null, emptyList(), false)
                    else {
                        var missingGroup = false
                        val rows = view.entries.mapNotNull { entry ->
                            val group = view.groups.firstOrNull { it.id == entry.group }
                            if (group == null) { missingGroup = true; null }
                            else SearchResultRow(source.copy, source.generation, view.name, group.name, entry)
                        }
                        SearchBatch(key(source), view.name, rows, missingGroup)
                    }
                } catch (cancelled: CancellationException) { throw cancelled }
                catch (_: Exception) { SearchBatch(key(source), null, emptyList(), source.permitted()) }
            } }.awaitAll()
            if (sequence != request || query != requestedQuery || scope != requestedScope || !showing) return@launch
            val allowed = sources().filter(OpenDocumentSource::permitted).map(::key).toSet()
            views.filter { it.source in allowed }.forEach { view -> view.name?.let { names[view.source] = it } }
            results = views.flatMap { it.rows }.filter { it.sourceKey in allowed }
                .sortedWith(order().comparator({ it.copy.database }, { it.entry }))
            error = views.any { it.failed && it.source in allowed } ||
                (requestedScope == SearchScope.CURRENT && requestedSources.none { key(it) in allowed })
            busy = false
        }
    }
    fun resort() {
        results = results.sortedWith(order().comparator({ it.copy.database }, { it.entry }))
    }
    /** Fast control checks only; no document read happens on the main thread. */
    fun reconcile() {
        val available = sources().filter(OpenDocumentSource::permitted).map(::key).toSet()
        results = results.filter { it.sourceKey in available }
        names.keys.toList().filter { it !in available }.forEach(names::remove)
        applicationRevisions.keys.retainAll(available)
        if (showing && querySources().map(::key).toSet() != observed) { previewEpoch++; refresh() }
    }
    fun close() {
        job?.cancel(); job = null; sequence++; previewEpoch++
        query = ""; scope = SearchScope.CURRENT; showing = false; results = emptyList()
        busy = false; error = false; returning = null; origin = null; observed = emptySet()
        names.clear(); applicationRevisions.clear()
    }
}
