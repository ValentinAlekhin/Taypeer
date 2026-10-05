package dev.taypeer.platform

import android.os.BadParcelableException
import android.os.Parcel
import android.os.Parcelable

internal const val SELECTED_CHUNK = 65536

/** One bounded plaintext reply; serialization clears the sender's JVM array. */
class SelectedChunk(val status: Int, val bytes: ByteArray?) : Parcelable {
    private constructor(parcel: Parcel) : this(parcel.readInt(), readChunk(parcel))
    override fun writeToParcel(parcel: Parcel, flags: Int) {
        try {
            if (bytes != null && bytes.size > SELECTED_CHUNK) throw BadParcelableException("Selected chunk exceeds limit")
            parcel.writeInt(status)
            parcel.writeInt(bytes?.size ?: -1)
            if (bytes != null) parcel.writeByteArray(bytes)
        } finally {
            if (flags and Parcelable.PARCELABLE_WRITE_RETURN_VALUE != 0) bytes?.fill(0)
        }
    }
    override fun describeContents(): Int = 0
    companion object {
        private fun readChunk(parcel: Parcel): ByteArray? {
            val size = parcel.readInt()
            if (size == -1) return null
            if (size !in 0..SELECTED_CHUNK) throw BadParcelableException("Invalid selected chunk length")
            val bytes = ByteArray(size)
            try { parcel.readByteArray(bytes); return bytes }
            catch (error: Exception) { bytes.fill(0); throw error }
        }
        @JvmField val CREATOR = object : Parcelable.Creator<SelectedChunk> {
            override fun createFromParcel(parcel: Parcel) = SelectedChunk(parcel)
            override fun newArray(size: Int): Array<SelectedChunk?> = arrayOfNulls(size)
        }
    }
}

/** A partial write count is separate from the explicit durability/error category. */
class SelectedWriteResult(val status: Int, val written: Int) : Parcelable {
    private constructor(parcel: Parcel) : this(parcel.readInt(), parcel.readInt())
    override fun writeToParcel(parcel: Parcel, flags: Int) { parcel.writeInt(status); parcel.writeInt(written) }
    override fun describeContents(): Int = 0
    companion object {
        @JvmField val CREATOR = object : Parcelable.Creator<SelectedWriteResult> {
            override fun createFromParcel(parcel: Parcel) = SelectedWriteResult(parcel)
            override fun newArray(size: Int): Array<SelectedWriteResult?> = arrayOfNulls(size)
        }
    }
}
