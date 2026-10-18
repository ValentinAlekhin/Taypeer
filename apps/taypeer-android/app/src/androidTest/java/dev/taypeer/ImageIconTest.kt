package dev.taypeer

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Color
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.taypeer.bridge.*
import dev.taypeer.platform.DocumentManager
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.ByteArrayOutputStream
import java.io.File
import java.util.UUID
import java.util.concurrent.atomic.AtomicInteger

/** PUBLIC originals cross real selected Binder capabilities and remain encrypted across reopen. */
@RunWith(AndroidJUnit4::class)
class ImageIconTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val password = "PUBLIC custom image master"
    private val svg = """<svg xmlns="http://www.w3.org/2000/svg" width="32" height="16"><rect width="32" height="16" fill="#14dc28"/></svg>""".toByteArray()

    private inline fun fixture(block: (DocumentManager) -> Unit) {
        val profile = File(context.noBackupFilesDir, "PUBLIC-image-icon-${UUID.randomUUID()}")
        try {
            Host.portable(profile.path).use { host ->
                host.setRelayPreference(RelayPreference.Disabled)
                DocumentManager(context, host).use { block(it) }
            }
        } finally { assertTrue(profile.deleteRecursively()) }
    }

    private class Input(private val bytes: ByteArray, private val refuse: Boolean = false,
        private val refuseLength: Boolean = false) : SelectedInput {
        val lengths = AtomicInteger()
        val reads = AtomicInteger()
        val closes = AtomicInteger()
        override fun length(): ULong {
            lengths.incrementAndGet()
            if (refuseLength) throw AndroidException.StorageIo()
            check(!refuse) { "Confirmed replay must not touch selected provider" }
            return bytes.size.toULong()
        }
        override fun read(offset: ULong, count: UInt): ByteArray {
            reads.incrementAndGet()
            check(!refuse)
            assertTrue(count <= 65536u)
            val start = offset.toInt()
            return bytes.copyOfRange(start, minOf(bytes.size, start + count.toInt()))
        }
        override fun close() { closes.incrementAndGet() }
    }

    private fun green(view: RenderedIcon?, origin: IconOrigin = IconOrigin.FILE) {
        assertNotNull(view)
        val image = BitmapFactory.decodeByteArray(view!!.png, 0, view.png.size)
        try {
            assertEquals(32, image.width); assertEquals(16, image.height)
            assertEquals(32u, view.width); assertEquals(16u, view.height)
            assertEquals(Color.rgb(20, 220, 40), image.getPixel(8, 8))
            assertEquals(origin, view.origin)
        } finally { image.recycle(); view.png.fill(0) }
    }

    @Test fun explicitHostDownloadIsTheOnlyRequestAndConfirmedReplayWorksAfterServerStops() = fixture { manager ->
        val session = manager.create("PUBLIC explicit icon network", null, newOperation(), password)
        val form = session.beginMetadata(FormKind.NEW_GROUP, null, null).form
        PublicIconServer(svg).use { server ->
            assertNull(session.iconPreview(IconScope.Form(form.draft)))
            assertEquals(0, server.requests.get())
            val operation = newOperation()
            val imported = session.downloadIcon(form.draft, IconDownload.ImageUrl(server.url), operation)
            assertEquals(1, server.requests.get())
            green(session.iconPreview(IconScope.Form(form.draft)), IconOrigin.IMAGE_URL)
            assertEquals(1, server.requests.get())
            server.close()
            val receipt = session.downloadIcon(form.draft, IconDownload.ImageUrl(server.url), operation)
            assertEquals(imported, receipt)
            assertEquals(1, server.requests.get())
            assertThrows(AndroidException::class.java) {
                session.downloadIcon(form.draft, IconDownload.ImageUrl(server.url + "?different=PUBLIC"), operation)
            }
            assertEquals(receipt, session.metadata(form.draft).form)
            assertEquals(1, server.requests.get())
            session.lock()
            assertThrows(AndroidException::class.java) {
                session.downloadIcon(form.draft, IconDownload.ImageUrl(server.url), newOperation())
            }
            assertEquals(1, server.requests.get())
            manager.close(session)
        }
    }

    @Test fun newGroupRetainsOriginalFaviconForLocalReselectAfterLucideWithoutAnotherRequest() = fixture { manager ->
        val session = manager.create("PUBLIC stored favicon", null, newOperation(), password)
        val draft = session.beginMetadata(FormKind.NEW_GROUP, null, null).form
        PublicIconServer(svg).use { server ->
            assertNull(session.storedFavicon(draft.draft))
            session.downloadIcon(draft.draft, IconDownload.Favicon(server.url), newOperation())
            assertEquals(2, server.requests.get()) // Explicit page and its fallback; no third-party service.
            val original = session.storedFavicon(draft.draft)!!
            assertTrue(original.available)
            green(session.iconPreview(IconScope.FormFavicon(draft.draft, original.identity)), IconOrigin.FAVICON)
            server.close()
            session.patchGroupIcon(draft.draft, TextEdit.Set("server"))
            assertNull(session.iconPreview(IconScope.Form(draft.draft)))
            assertEquals(original, session.storedFavicon(draft.draft))
            val operation = newOperation()
            val reselected = session.selectStoredFavicon(draft.draft, original.identity, operation)
            val preview = session.iconPreview(IconScope.Form(draft.draft))!!
            assertEquals(original.identity, preview.identity)
            green(preview, IconOrigin.FAVICON)
            assertEquals(reselected, session.selectStoredFavicon(draft.draft, original.identity, operation))
            assertEquals(2, server.requests.get())
            assertTrue(session.overview("").groups.isEmpty())
            val completed = session.patchMetadata(draft.draft, FormKind.NEW_GROUP, TextEdit.Set("PUBLIC favicon group"), TextEdit.Keep)
            val group = session.save(completed.form.draft, completed.form.revision, newOperation()).form.target!!
            assertEquals(1, session.history(FormKind.GROUP, group).size)
            assertEquals(IconView.Image, session.overview("").groups.single().icon)

            val other = session.beginMetadata(FormKind.NEW_GROUP, null, null)
            assertThrows(AndroidException::class.java) {
                session.selectStoredFavicon(other.form.draft, original.identity, newOperation())
            }
            assertEquals(other, session.metadata(other.form.draft))
            assertEquals(2, server.requests.get())
            assertEquals(1, session.history(FormKind.GROUP, group).size)
        }
    }

    @Test fun unfinishedGroupStagesImageAndOneSaveCommitsTextWithIconAcrossReopen() = fixture { manager ->
        val session = manager.create("PUBLIC group image", null, newOperation(), password)
        val form = session.beginMetadata(FormKind.NEW_GROUP, null, null).form
        val operation = newOperation()
        val input = Input(svg)
        val imported = session.importIcon(form.draft, operation, input)
        assertEquals(form.draft, imported.draft)
        assertTrue(imported.dirty)
        assertEquals(1, input.lengths.get()); assertTrue(input.reads.get() > 0)
        assertEquals(1, input.closes.get())
        assertTrue(session.overview("").groups.isEmpty())
        assertEquals("", session.metadata(form.draft).name)
        green(session.iconPreview(IconScope.Form(form.draft)))
        val completed = session.patchMetadata(form.draft, FormKind.NEW_GROUP,
            TextEdit.Set("PUBLIC image group"), TextEdit.Set("PUBLIC same causal description"))
        val receipt = session.save(form.draft, completed.form.revision, newOperation())
        val group = receipt.form.target!!
        assertEquals(SaveState.SAVED, receipt.state)
        assertEquals(1, session.history(FormKind.GROUP, group).size)
        assertEquals(IconView.Image, session.overview("").groups.single().icon)
        green(session.iconPreview(IconScope.Group(group)))
        val repeated = Input(byteArrayOf(), refuse = true)
        session.importIcon(form.draft, operation, repeated)
        assertEquals(0, repeated.lengths.get()); assertEquals(0, repeated.reads.get())
        assertEquals(1, repeated.closes.get())
        val path = manager.catalog().single().path
        assertFalse(File(path).readBytes().toString(Charsets.ISO_8859_1).contains(svg.toString(Charsets.UTF_8)))
        session.lock(); manager.close(session)
        val reopened = manager.open(path, password)
        assertEquals(1, reopened.history(FormKind.GROUP, group).size)
        assertEquals("PUBLIC same causal description", reopened.overview("").groups.single().description)
        green(reopened.iconPreview(IconScope.Group(group)))
        val replay = Input(byteArrayOf(), refuse = true)
        reopened.importIcon(form.draft, operation, replay)
        assertEquals(0, replay.lengths.get()); assertEquals(0, replay.reads.get())
        assertEquals(1, replay.closes.get())
        assertEquals(1, reopened.history(FormKind.GROUP, group).size)
    }

    @Test fun largeOriginalDoesNotEnterEditorJsonAndHistoricalImageDoesNotReplaceCurrentLucide() = fixture { manager ->
        val session = manager.create("PUBLIC large image", null, newOperation(), password)
        val group = session.beginMetadata(FormKind.NEW_GROUP, null, null)
        val named = session.patchMetadata(group.form.draft, FormKind.NEW_GROUP, TextEdit.Set("PUBLIC group"), TextEdit.Keep)
        session.save(named.form.draft, named.form.revision, newOperation())
        val draft = session.beginCreateEntry(named.form.target!!)
        val titled = session.patchEntry(EntryTextField.TITLE, TextEdit.Set("PUBLIC image entry"))
        val bitmap = Bitmap.createBitmap(400, 400, Bitmap.Config.ARGB_8888)
        val random = java.util.Random(713)
        for (y in 0 until 400) for (x in 0 until 400) bitmap.setPixel(x, y, Color.rgb(random.nextInt(256), random.nextInt(256), random.nextInt(256)))
        val encoded = ByteArrayOutputStream()
        bitmap.compress(Bitmap.CompressFormat.PNG, 100, encoded); bitmap.recycle()
        val original = encoded.toByteArray()
        assertTrue(original.size > 400_000); assertTrue(original.size <= 1_048_576)
        val imported = session.importIcon(draft.form.draft, newOperation(), Input(original))
        assertEquals(titled.title, session.editor().title) // Original bytes would overflow the JSON projection.
        assertEquals(imported, session.editor().form)
        val first = session.save(imported.draft, imported.revision, newOperation())
        val entry = first.form.target!!
        val revision = session.history(FormKind.ENTRY, entry).single().id
        val saved = session.iconPreview(IconScope.Entry(entry))!!
        assertTrue(saved.width <= 256u && saved.height <= 256u); saved.png.fill(0)
        val bundled = session.setIcon("server", newOperation())
        assertNull(session.iconPreview(IconScope.Form(bundled.form.draft)))
        session.save(bundled.form.draft, bundled.form.revision, newOperation())
        assertNull(session.iconPreview(IconScope.Entry(entry)))
        val historical = session.iconPreview(IconScope.Revision(entry, revision))!!
        assertEquals(IconOrigin.FILE, historical.origin); historical.png.fill(0)
        assertEquals(2, session.history(FormKind.ENTRY, entry).size)
        val path = manager.catalog().single().path
        val encrypted = File(path).readBytes()
        assertFalse(encrypted.toString(Charsets.ISO_8859_1).contains(original.copyOfRange(100, 300).toString(Charsets.ISO_8859_1)))
        session.lock(); manager.close(session)
        val reopened = manager.open(path, password)
        assertNull(reopened.iconPreview(IconScope.Entry(entry)))
        reopened.iconPreview(IconScope.Revision(entry, revision))!!.png.fill(0)
        assertEquals(2, reopened.history(FormKind.ENTRY, entry).size)
        original.fill(0)
    }

    @Test fun selectedProviderLengthFailureIsDefinitiveAndPreservesExactEncryptedForm() = fixture { manager ->
        val session = manager.create("PUBLIC provider length refusal", null, newOperation(), password)
        val draft = session.beginMetadata(FormKind.NEW_GROUP, null, null).form
        val original = session.metadata(draft.draft)
        val projection = session.overview("")
        val file = File(manager.catalog().single().path)
        val encrypted = file.readBytes()
        val rejected = Input(svg, refuseLength = true)

        val error = assertThrows(AndroidException.Icon::class.java) {
            session.importIcon(draft.draft, newOperation(), rejected)
        }

        assertEquals(IconFailure.FILE, error.failure)
        assertEquals(1, rejected.lengths.get()); assertEquals(0, rejected.reads.get())
        assertEquals(1, rejected.closes.get())
        assertEquals(original, session.metadata(draft.draft))
        assertEquals(projection, session.overview(""))
        assertArrayEquals(encrypted, file.readBytes())
        assertNull(session.iconPreview(IconScope.Form(draft.draft)))
        assertEquals(SessionAccess.OPEN, session.access())
    }

    @Test fun refusedImagePreservesExactDraftAndRevokedSessionDoesNotOpenProvider() = fixture { manager ->
        val session = manager.create("PUBLIC refusal image", null, newOperation(), password)
        val draft = session.beginMetadata(FormKind.NEW_GROUP, null, null).form
        val original = session.metadata(draft.draft)
        val invalid = Input("PUBLIC invalid image".toByteArray())
        val error = assertThrows(AndroidException.Icon::class.java) {
            session.importIcon(draft.draft, newOperation(), invalid)
        }
        assertEquals(IconFailure.INVALID_IMAGE, error.failure)
        assertEquals(1, invalid.closes.get())
        assertEquals(original, session.metadata(draft.draft))
        assertTrue(session.overview("").groups.isEmpty())
        session.lock()
        val revoked = Input(svg)
        assertThrows(AndroidException::class.java) { session.importIcon(draft.draft, newOperation(), revoked) }
        assertEquals(0, revoked.lengths.get()); assertEquals(0, revoked.reads.get())
        assertEquals(1, revoked.closes.get())
        assertThrows(AndroidException::class.java) { session.iconPreview(IconScope.Form(draft.draft)) }
        manager.close(session)
    }
}
