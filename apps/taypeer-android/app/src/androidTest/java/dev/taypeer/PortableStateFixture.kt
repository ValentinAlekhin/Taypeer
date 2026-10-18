package dev.taypeer

import org.junit.Assert.*
import java.io.File

/** Exact encrypted host-state publications in this test's dedicated PUBLIC profile. */
internal fun publicPortableStates(profile: File): Map<File, ByteArray> =
    profile.walkTopDown().filter { it.isFile && it.name == "state" }.associateWith { it.readBytes() }.also {
        assertTrue("PUBLIC portable profile must have actual encrypted state", it.isNotEmpty())
    }

internal fun assertPublicPortableStatesUnchanged(before: Map<File, ByteArray>) {
    before.forEach { (file, bytes) -> assertArrayEquals("PUBLIC inspection must not publish local access state", bytes, file.readBytes()) }
}
