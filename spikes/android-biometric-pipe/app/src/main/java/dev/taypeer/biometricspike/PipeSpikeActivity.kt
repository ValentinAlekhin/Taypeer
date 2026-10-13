package dev.taypeer.biometricspike

import android.os.Bundle
import android.widget.TextView
import androidx.fragment.app.FragmentActivity

/** PUBLIC-only harness; no document, profile, worker, network or saved secret. */
class PipeSpikeActivity : FragmentActivity() {
    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        setContentView(TextView(this).apply { text = "PUBLIC biometric pipe check" })
    }
}
