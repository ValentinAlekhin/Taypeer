package dev.taypeer.platform

import android.os.Parcel
import android.os.ParcelFileDescriptor
import android.os.Parcelable

/** Explicit private descriptor result; absent collection differs from allocation failure. */
class CipherFileResult(val status: Int, val file: ParcelFileDescriptor?) : Parcelable {
    private constructor(parcel: Parcel) : this(parcel.readInt(), if (parcel.readInt() == 0) null else ParcelFileDescriptor.CREATOR.createFromParcel(parcel))
    override fun writeToParcel(parcel: Parcel, flags: Int) {
        parcel.writeInt(status)
        parcel.writeInt(if (file == null) 0 else 1)
        file?.writeToParcel(parcel, flags)
    }
    override fun describeContents(): Int = if (file == null) 0 else Parcelable.CONTENTS_FILE_DESCRIPTOR
    companion object {
        @JvmField val CREATOR = object : Parcelable.Creator<CipherFileResult> {
            override fun createFromParcel(parcel: Parcel) = CipherFileResult(parcel)
            override fun newArray(size: Int): Array<CipherFileResult?> = arrayOfNulls(size)
        }
    }
}

/** Private authenticated author capability; never a UI DTO, saved-state value or Bundle. */
class CipherAuthorResult(val status: Int, val seed: ByteArray?) : Parcelable {
    private constructor(parcel: Parcel) : this(parcel.readInt(), parcel.createByteArray())
    override fun writeToParcel(parcel: Parcel, flags: Int) {
        parcel.writeInt(status)
        parcel.writeByteArray(seed)
        if (flags and Parcelable.PARCELABLE_WRITE_RETURN_VALUE != 0) seed?.fill(0)
    }
    override fun describeContents(): Int = 0
    companion object {
        @JvmField val CREATOR = object : Parcelable.Creator<CipherAuthorResult> {
            override fun createFromParcel(parcel: Parcel) = CipherAuthorResult(parcel)
            override fun newArray(size: Int): Array<CipherAuthorResult?> = arrayOfNulls(size)
        }
    }
}
