package dev.taypeer.qr

import android.Manifest
import android.content.pm.PackageManager
import android.util.Size
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.Preview
import androidx.camera.core.resolutionselector.ResolutionSelector
import androidx.camera.core.resolutionselector.ResolutionStrategy
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import dev.taypeer.R
import dev.taypeer.PhoneBar
import dev.taypeer.FlatAction
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

/** Permission is requested only after the user has opened the scanner. No frames leave the device. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun QrScannerScreen(onCode: (String) -> Unit, onClose: () -> Unit) {
    val context = LocalContext.current
    var granted by remember { mutableStateOf(context.checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) }
    var unavailable by remember { mutableStateOf(false) }
    val permission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted = it }
    val lifecycle = LocalLifecycleOwner.current
    DisposableEffect(lifecycle) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_RESUME) granted = context.checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED
        }
        lifecycle.lifecycle.addObserver(observer)
        onDispose { lifecycle.lifecycle.removeObserver(observer) }
    }
    LaunchedEffect(Unit) { if (!granted) permission.launch(Manifest.permission.CAMERA) }
    BackHandler(onBack = onClose)
    Scaffold(topBar = { PhoneBar(stringResource(R.string.scan_qr_title), back = onClose) }, bottomBar = {
        FlatAction(R.string.enter_invitation_code, Modifier.fillMaxWidth().navigationBarsPadding().padding(16.dp).testTag("enter-qr-code"), action = onClose)
    }) { insets ->
        Column(Modifier.fillMaxSize().padding(insets).padding(16.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Text(stringResource(R.string.scan_qr_hint))
            if (granted && !unavailable) {
                Box(Modifier.fillMaxWidth().weight(1f).testTag("qr-camera-preview"), contentAlignment = Alignment.Center) {
                    CameraPreview(onCode, { unavailable = true }, Modifier.fillMaxSize())
                    Box(Modifier.size(240.dp).border(2.dp, Color.White, RoundedCornerShape(4.dp)))
                }
            } else {
                Text(stringResource(if (unavailable) R.string.camera_unavailable else R.string.camera_permission_message), Modifier.testTag("qr-camera-status"))
                if (!unavailable) FlatAction(R.string.allow_camera, Modifier.fillMaxWidth().testTag("request-camera")) { permission.launch(Manifest.permission.CAMERA) }
            }
        }
    }
}

@Composable
private fun CameraPreview(onCode: (String) -> Unit, onUnavailable: () -> Unit, modifier: Modifier) {
    val context = LocalContext.current
    val owner = LocalLifecycleOwner.current
    val codeCallback by rememberUpdatedState(onCode)
    val errorCallback by rememberUpdatedState(onUnavailable)
    val previewView = remember(context) { PreviewView(context).apply { implementationMode = PreviewView.ImplementationMode.COMPATIBLE } }
    AndroidView(factory = { previewView }, modifier = modifier)
    DisposableEffect(previewView, owner) {
        val active = AtomicBoolean(true)
        val executor = Executors.newSingleThreadExecutor()
        val main = ContextCompat.getMainExecutor(context)
        val preview = Preview.Builder().build().apply { setSurfaceProvider(previewView.surfaceProvider) }
        val analysis = ImageAnalysis.Builder()
            .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
            .setResolutionSelector(ResolutionSelector.Builder().setResolutionStrategy(ResolutionStrategy(Size(1280, 720), ResolutionStrategy.FALLBACK_RULE_CLOSEST_LOWER_THEN_HIGHER)).build())
            .build()
        val analyzer = QrFrameAnalyzer(main, { owner.lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED) }, { codeCallback(it) })
        analysis.setAnalyzer(executor, analyzer)
        val future = ProcessCameraProvider.getInstance(context)
        var provider: ProcessCameraProvider? = null
        future.addListener({
            if (active.get()) try {
                provider = future.get()
                val camera = provider!!
                val selector = when {
                    camera.hasCamera(CameraSelector.DEFAULT_BACK_CAMERA) -> CameraSelector.DEFAULT_BACK_CAMERA
                    camera.hasCamera(CameraSelector.DEFAULT_FRONT_CAMERA) -> CameraSelector.DEFAULT_FRONT_CAMERA
                    else -> throw IllegalStateException("No camera")
                }
                camera.bindToLifecycle(owner, selector, preview, analysis)
            } catch (_: Exception) { if (active.get()) errorCallback() }
        }, main)
        onDispose {
            active.set(false)
            analyzer.stop()
            analysis.clearAnalyzer()
            provider?.unbind(preview, analysis)
            executor.shutdown()
        }
    }
}
