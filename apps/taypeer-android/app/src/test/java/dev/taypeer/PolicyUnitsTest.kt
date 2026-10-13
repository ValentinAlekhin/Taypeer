package dev.taypeer

import org.junit.Assert.assertEquals
import org.junit.Test
import java.util.Locale

class PolicyUnitsTest {
    @Test fun compactUsageIsLocalizedWithoutRoundingEditableAmounts() {
        val bytes = 2_516_582uL
        val russian = Locale.forLanguageTag("ru-RU")
        assertEquals("2.4", PolicyUnits.usageMib(bytes, Locale.ENGLISH))
        assertEquals("2,4", PolicyUnits.usageMib(bytes, russian))
        assertEquals("2.3999996185302734375", PolicyUnits.editableMib(bytes))
        assertEquals(bytes, PolicyUnits.parseMib(PolicyUnits.editableMib(bytes)))
        assertEquals(bytes, PolicyUnits.parseMib(PolicyUnits.mib(bytes, russian)))
        assertEquals(ULong.MAX_VALUE, PolicyUnits.parseMib(PolicyUnits.editableMib(ULong.MAX_VALUE)))
    }

    @Test fun smallPositiveUsageIsNotDisplayedAsZero() {
        assertEquals("0", PolicyUnits.usageMib(0uL, Locale.ENGLISH))
        assertEquals("<0.01", PolicyUnits.usageMib(1uL, Locale.ENGLISH))
        assertEquals("<0,01", PolicyUnits.usageMib(1uL, Locale.forLanguageTag("ru-RU")))
        assertEquals("0.00000095367431640625", PolicyUnits.editableMib(1uL))
        assertEquals(1uL, PolicyUnits.parseMib(PolicyUnits.editableMib(1uL)))
    }
}
