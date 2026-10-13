package dev.taypeer

import android.content.Context
import dev.taypeer.bridge.DocumentSession
import dev.taypeer.bridge.LocalForm

/** Internal dependency for stage acknowledgement tests; the default always calls real Native. */
internal interface IconStageExecutor {
    fun pin(session: DocumentSession): LocalForm
    fun execute(session: DocumentSession, request: IconRequest): LocalForm
}

internal class NativeIconStageExecutor(context: Context) : IconStageExecutor {
    private val context = context.applicationContext
    override fun pin(session: DocumentSession): LocalForm = session.pinActiveForm()
    override fun execute(session: DocumentSession, request: IconRequest): LocalForm = when (val source = request.source) {
        is IconSource.File -> session.importIcon(request.target.draft, request.operation, IconSelectedInput(context, source.uri))
        is IconSource.Download -> session.downloadIcon(request.target.draft, source.value, request.operation)
        is IconSource.Stored -> session.selectStoredFavicon(request.target.draft, source.identity, request.operation)
    }
}
