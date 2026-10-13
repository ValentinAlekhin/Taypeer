package dev.taypeer.platform

import android.os.BadParcelableException
import android.os.Parcel
import android.os.Parcelable
import dev.taypeer.bridge.BiometricInputSpec
import dev.taypeer.bridge.BiometricPurpose

/** Bounded public stamps for the private input port. No key bytes or numeric FD. */
class BiometricInputParcel(val scope: BiometricInputSpec) : Parcelable {
    init { validate(scope) }
    private constructor(parcel: Parcel) : this(read(parcel))

    override fun writeToParcel(parcel: Parcel, flags: Int) {
        validate(scope)
        parcel.writeString(scope.namespace)
        parcel.writeString(scope.workingCopy)
        parcel.writeString(scope.database)
        parcel.writeString(scope.root)
        parcel.writeLong(scope.epoch.toLong())
        parcel.writeString(scope.control)
        parcel.writeString(scope.snapshot)
        parcel.writeString(scope.checkpoint)
        parcel.writeLong(scope.generation.toLong())
        parcel.writeString(scope.attempt)
        parcel.writeString(scope.input)
        parcel.writeString(when (scope.purpose) {
            BiometricPurpose.ENROLLMENT_PROOF -> "enrollment_proof"
            BiometricPurpose.KEYED_BOOT -> "keyed_boot"
        })
    }
    override fun describeContents(): Int = 0

    companion object {
        private fun invalid(): Nothing = throw BadParcelableException("Invalid biometric input scope")
        private fun validate(scope: BiometricInputSpec) {
            for (digest in listOf(scope.namespace, scope.workingCopy, scope.root, scope.control,
                scope.snapshot, scope.checkpoint, scope.attempt, scope.input)) {
                if (!digest.matches(Regex("[0-9a-f]{64}"))) invalid()
            }
            if (scope.database.isEmpty() || scope.database.toByteArray(Charsets.UTF_8).size > 128 ||
                scope.generation == 0uL) invalid()
        }
        private fun read(parcel: Parcel): BiometricInputSpec {
            fun text() = parcel.readString() ?: invalid()
            val namespace = text()
            val copy = text()
            val database = text()
            val root = text()
            val epoch = parcel.readLong().toULong()
            val control = text()
            val snapshot = text()
            val checkpoint = text()
            val generation = parcel.readLong().toULong()
            val attempt = text()
            val input = text()
            val purpose = when (text()) {
                "enrollment_proof" -> BiometricPurpose.ENROLLMENT_PROOF
                "keyed_boot" -> BiometricPurpose.KEYED_BOOT
                else -> invalid()
            }
            return BiometricInputSpec(namespace, copy, database, root, epoch, control, snapshot,
                checkpoint, generation, attempt, input, purpose)
        }
        @JvmField val CREATOR = object : Parcelable.Creator<BiometricInputParcel> {
            override fun createFromParcel(parcel: Parcel) = BiometricInputParcel(parcel)
            override fun newArray(size: Int): Array<BiometricInputParcel?> = arrayOfNulls(size)
        }
    }
}
