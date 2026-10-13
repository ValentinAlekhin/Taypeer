package dev.taypeer

import java.math.BigDecimal
import java.math.RoundingMode
import java.text.DecimalFormatSymbols
import java.util.Locale

/** Presentation conversions only. Rust owns quota and KDF target validation. */
internal object PolicyUnits {
    private val mib = BigDecimal(1_048_576)
    fun editableMib(bytes: ULong): String = BigDecimal(bytes.toString()).divide(mib).stripTrailingZeros().toPlainString()
    fun parseMib(raw: String): ULong? = try {
        BigDecimal(raw.replace(',', '.')).multiply(mib).toBigIntegerExact().toString().toULongOrNull()
    } catch (_: NumberFormatException) { null }
      catch (_: ArithmeticException) { null }
    fun mib(bytes: ULong, locale: Locale): String = localized(editableMib(bytes), locale)
    /** Read-only usage is compact; editable amounts always retain every byte. */
    fun usageMib(bytes: ULong, locale: Locale): String {
        val rounded = BigDecimal(bytes.toString()).divide(mib, 2, RoundingMode.HALF_UP)
        if (bytes > 0uL && rounded.signum() == 0) return "<${localized("0.01", locale)}"
        return localized(rounded.stripTrailingZeros().toPlainString(), locale)
    }
    fun seconds(milliseconds: ULong, locale: Locale): String = localized(
        BigDecimal(milliseconds.toString()).divide(BigDecimal(1000)).stripTrailingZeros().toPlainString(), locale)
    private fun localized(value: String, locale: Locale) = value.replace('.', DecimalFormatSymbols.getInstance(locale).decimalSeparator)
}
