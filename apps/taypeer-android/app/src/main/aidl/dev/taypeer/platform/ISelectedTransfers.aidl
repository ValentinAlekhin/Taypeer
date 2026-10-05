package dev.taypeer.platform;
import dev.taypeer.platform.SelectedChunk;
import dev.taypeer.platform.SelectedWriteResult;

/** Ephemeral, explicitly selected plaintext capabilities; never ciphertext storage access. */
interface ISelectedTransfers {
    SelectedChunk read(long id, long offset, int count);
    SelectedWriteResult write(long id, long offset, in byte[] bytes);
    int finish(long id);
}
