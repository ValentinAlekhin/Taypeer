package dev.taypeer

import androidx.compose.runtime.staticCompositionLocalOf

/** A display invalidation only; entered values remain owned by their current form. */
internal val LocalSecretMaskEpoch = staticCompositionLocalOf { 0L }
