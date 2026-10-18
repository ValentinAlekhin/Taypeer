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
