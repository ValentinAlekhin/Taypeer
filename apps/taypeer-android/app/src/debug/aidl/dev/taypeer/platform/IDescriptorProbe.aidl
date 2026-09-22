package dev.taypeer.platform;
import android.os.ParcelFileDescriptor;
import dev.taypeer.platform.ITemporaryFiles;
/** Exercises real storage on a transferred immutable archive and anonymous staging FDs. */
interface IDescriptorProbe {
    long verify(in ParcelFileDescriptor archive, ITemporaryFiles files);
}
