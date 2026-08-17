package dev.prismaemu.app

import androidx.activity.compose.BackHandler
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlin.math.cos
import kotlin.math.sin

private enum class InspectorTab {
    Preview,
    Ir,
    Arm64,
}

private data class TraceStage(
    val name: String,
    val detail: String,
    val elapsed: String,
    val result: String,
)

private val traceStages = listOf(
    TraceStage("Decode x86-64", "48 89 E5 0F 58 C1", "0.00", "6 B"),
    TraceStage("Build SSA IR", "typed block_401000", "0.18", "12 ops"),
    TraceStage("Optimize", "CSE · fold · DCE", "0.41", "−3 ops"),
    TraceStage("Lower ARM64", "AArch64 + NEON", "0.72", "32 B"),
)

private val sampleIr = """
block_401000:
  %0 = load_gpr rbp
  store_gpr rsp, %0
  %1 = load_vec xmm0
  %2 = load_vec xmm1
  %3 = vec_add.f32x4 %1, %2
  store_vec xmm0, %3
  return %next_rip
""".trimIndent()

private val sampleArm64 = """
0x0000  stp   x29, x30, [sp, #-16]!
0x0004  mov   x29, sp
0x0008  ldr   q0, [x19, #0x100]
0x000c  ldr   q1, [x19, #0x110]
0x0010  fadd  v0.4s, v0.4s, v1.4s
0x0014  str   q0, [x19, #0x100]
0x0018  ldp   x29, x30, [sp], #16
0x001c  ret
""".trimIndent()

@Composable
fun TranslatorDemo(onDismiss: () -> Unit) {
    var selectedTab by remember { mutableStateOf(InspectorTab.Preview) }
    var runKey by remember { mutableStateOf(0) }
    var activeStage by remember { mutableStateOf(0) }
    var isRunning by remember { mutableStateOf(true) }

    BackHandler(onBack = onDismiss)

    LaunchedEffect(runKey) {
        isRunning = true
        traceStages.indices.forEach { index ->
            activeStage = index
            delay(420)
        }
        activeStage = traceStages.size
        isRunning = false
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(PrismaBackground)
            .statusBarsPadding(),
    ) {
        InspectorHeader(
            isRunning = isRunning,
            onBack = onDismiss,
            onReplay = { runKey += 1 },
        )
        InspectorTabs(selected = selectedTab, onSelect = { selectedTab = it })
        when (selectedTab) {
            InspectorTab.Preview -> PreviewPane(
                activeStage = activeStage,
                isRunning = isRunning,
            )
            InspectorTab.Ir -> CodePane(
                title = "SSA intermediate representation",
                subtitle = if (LocalPrismaLanguage.current.tag == "es") {
                    "Bloque de muestra · normalizado tras decodificar"
                } else {
                    "Sample block · normalized after decode"
                },
                code = sampleIr,
            )
            InspectorTab.Arm64 -> CodePane(
                title = "ARM64 lowering",
                subtitle = if (LocalPrismaLanguage.current.tag == "es") {
                    "Salida de muestra · no ejecutada en este AVD x86-64"
                } else {
                    "Sample output · not executed on this x86-64 AVD"
                },
                code = sampleArm64,
            )
        }
    }
}

@Composable
private fun InspectorHeader(
    isRunning: Boolean,
    onBack: () -> Unit,
    onReplay: () -> Unit,
) {
    val copy = technicalCopy()
    PrismaTopBar(
        title = "cube3d.exe",
        subtitle = if (isRunning) copy.translatingSample else copy.traceComplete,
        statusColor = if (isRunning) PrismaWarning else PrismaSuccess,
        onBack = onBack,
    ) {
        TextButton(onClick = onReplay, enabled = !isRunning) {
            Icon(
                imageVector = Icons.Default.Refresh,
                contentDescription = null,
                modifier = Modifier.size(18.dp),
                tint = if (isRunning) PrismaTextMuted else PrismaPrimary,
            )
            Spacer(modifier = Modifier.width(PrismaSpacing.Sm))
            Text(
                text = copy.replay,
                color = if (isRunning) PrismaTextMuted else PrismaPrimary,
            )
        }
    }
}

@Composable
private fun InspectorTabs(
    selected: InspectorTab,
    onSelect: (InspectorTab) -> Unit,
) {
    val copy = technicalCopy()
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .height(48.dp),
    ) {
        InspectorTab.entries.forEach { tab ->
            Column(
                modifier = Modifier
                    .weight(1f)
                    .fillMaxSize()
                    .clickable { onSelect(tab) },
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.Bottom,
            ) {
                Box(
                    modifier = Modifier.weight(1f),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        text = when (tab) {
                            InspectorTab.Preview -> copy.preview
                            InspectorTab.Ir -> "IR"
                            InspectorTab.Arm64 -> "ARM64"
                        },
                        color = if (selected == tab) PrismaTextPrimary else PrismaTextMuted,
                        style = PrismaTypography.labelLarge,
                        fontWeight = if (selected == tab) FontWeight.SemiBold else FontWeight.Normal,
                    )
                }
                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .height(2.dp)
                        .background(if (selected == tab) PrismaPrimary else PrismaBorder),
                )
            }
        }
    }
}

@Composable
private fun PreviewPane(
    activeStage: Int,
    isRunning: Boolean,
) {
    val copy = technicalCopy()
    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .navigationBarsPadding(),
    ) {
        RenderViewport(isRunning = isRunning)
        RuntimeFacts()
        HorizontalDivider(color = PrismaBorder)
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = PrismaSpacing.Lg, vertical = PrismaSpacing.Md),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = copy.translationTrace,
                color = PrismaTextPrimary,
                style = PrismaTypography.titleMedium,
                fontWeight = FontWeight.SemiBold,
                modifier = Modifier.weight(1f),
            )
            Text(
                text = copy.elapsedResult,
                color = PrismaTextMuted,
                style = PrismaTypography.labelSmall,
                fontFamily = FontFamily.Monospace,
            )
        }
        traceStages.forEachIndexed { index, stage ->
            TraceRow(
                stage = stage,
                isActive = isRunning && index == activeStage,
                isComplete = !isRunning || index < activeStage,
            )
            HorizontalDivider(
                modifier = Modifier.padding(start = 72.dp),
                color = PrismaBorder,
            )
        }
        Row(
            modifier = Modifier.padding(PrismaSpacing.Lg),
            verticalAlignment = Alignment.Top,
            horizontalArrangement = Arrangement.spacedBy(PrismaSpacing.Md),
        ) {
            Box(
                modifier = Modifier
                    .padding(top = 6.dp)
                    .size(6.dp)
                    .background(PrismaWarning, CircleShape),
            )
            Text(
                text = copy.previewOnly,
                color = PrismaTextMuted,
                style = PrismaTypography.bodySmall,
            )
        }
    }
}

@Composable
private fun RenderViewport(isRunning: Boolean) {
    val copy = technicalCopy()
    val transition = rememberInfiniteTransition(label = "viewport")
    val angle by transition.animateFloat(
        initialValue = 0f,
        targetValue = 6.28318f,
        animationSpec = infiniteRepeatable(
            animation = tween(
                durationMillis = if (isRunning) 2800 else 5600,
                easing = LinearEasing,
            ),
            repeatMode = RepeatMode.Restart,
        ),
        label = "cube-rotation",
    )

    Box(
        modifier = Modifier
            .fillMaxWidth()
            .height(286.dp)
            .padding(PrismaSpacing.Lg)
            .background(PrismaInspector.Panel, RoundedCornerShape(PrismaInspector.Corner))
            .semantics { contentDescription = copy.sampleViewport },
    ) {
        Canvas(modifier = Modifier.fillMaxSize()) {
            val gridStep = 32.dp.toPx()
            var gridX = 0f
            while (gridX <= size.width) {
                drawLine(
                    color = PrismaInspector.Grid.copy(alpha = 0.48f),
                    start = Offset(gridX, 0f),
                    end = Offset(gridX, size.height),
                    strokeWidth = 1.dp.toPx(),
                )
                gridX += gridStep
            }
            var gridY = 0f
            while (gridY <= size.height) {
                drawLine(
                    color = PrismaInspector.Grid.copy(alpha = 0.48f),
                    start = Offset(0f, gridY),
                    end = Offset(size.width, gridY),
                    strokeWidth = 1.dp.toPx(),
                )
                gridY += gridStep
            }

            val vertices = listOf(
                Triple(-1f, -1f, -1f), Triple(1f, -1f, -1f),
                Triple(1f, 1f, -1f), Triple(-1f, 1f, -1f),
                Triple(-1f, -1f, 1f), Triple(1f, -1f, 1f),
                Triple(1f, 1f, 1f), Triple(-1f, 1f, 1f),
            )
            val edges = listOf(
                0 to 1, 1 to 2, 2 to 3, 3 to 0,
                4 to 5, 5 to 6, 6 to 7, 7 to 4,
                0 to 4, 1 to 5, 2 to 6, 3 to 7,
            )
            val scale = size.minDimension * 0.26f
            val projected = vertices.map { (x, y, z) ->
                val rotatedX = x * cos(angle) - z * sin(angle)
                val rotatedZ = x * sin(angle) + z * cos(angle)
                val tiltedY = y * cos(0.42f) - rotatedZ * sin(0.42f)
                val depth = 4f + y * sin(0.42f) + rotatedZ * cos(0.42f)
                val perspective = 4f / depth
                Offset(
                    x = center.x + rotatedX * scale * perspective,
                    y = center.y + tiltedY * scale * perspective,
                )
            }
            edges.forEach { edge ->
                drawLine(
                    color = PrismaPrimary,
                    start = projected[edge.first],
                    end = projected[edge.second],
                    strokeWidth = 2.dp.toPx(),
                )
            }
            projected.forEach { point ->
                drawCircle(
                    color = PrismaPrimary,
                    radius = 2.5.dp.toPx(),
                    center = point,
                )
            }
        }
        Text(
            text = "Viewport 1280 × 720",
            modifier = Modifier
                .align(Alignment.TopStart)
                .padding(PrismaSpacing.Md),
            color = PrismaTextMuted,
            style = PrismaTypography.labelSmall,
            fontFamily = FontFamily.Monospace,
        )
        Text(
            text = "60 fps",
            modifier = Modifier
                .align(Alignment.TopEnd)
                .padding(PrismaSpacing.Md),
            color = PrismaSuccess,
            style = PrismaTypography.labelSmall,
            fontFamily = FontFamily.Monospace,
        )
        Text(
            text = copy.sampleFrame,
            modifier = Modifier
                .align(Alignment.BottomStart)
                .padding(PrismaSpacing.Md),
            color = PrismaTextMuted,
            style = PrismaTypography.labelSmall,
            fontFamily = FontFamily.Monospace,
        )
    }
}

@Composable
private fun RuntimeFacts() {
    val copy = technicalCopy()
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = PrismaSpacing.Lg, vertical = PrismaSpacing.Sm),
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Fact(copy.guest, "x86-64")
        Fact(copy.hostTarget, "ARM64")
        Fact(copy.block, "0x401000")
        Fact(copy.cache, "miss")
    }
}

@Composable
private fun Fact(label: String, value: String) {
    Column {
        Text(
            text = label,
            color = PrismaTextMuted,
            style = PrismaTypography.labelSmall,
        )
        Text(
            text = value,
            color = PrismaTextPrimary,
            style = PrismaTypography.bodySmall,
            fontFamily = FontFamily.Monospace,
            fontWeight = FontWeight.Medium,
        )
    }
}

@Composable
private fun TraceRow(
    stage: TraceStage,
    isActive: Boolean,
    isComplete: Boolean,
) {
    val copy = technicalCopy()
    val stateColor = when {
        isActive -> PrismaPrimary
        isComplete -> PrismaTextSecondary
        else -> PrismaTextMuted
    }
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .height(PrismaInspector.RowHeight)
            .background(if (isActive) PrismaInspector.Selection else PrismaBackground),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(
            modifier = Modifier
                .width(3.dp)
                .height(PrismaInspector.RowHeight)
                .background(if (isActive) PrismaPrimary else PrismaBackground),
        )
        Text(
            text = stage.elapsed,
            modifier = Modifier
                .width(69.dp)
                .padding(start = PrismaSpacing.Md),
            color = PrismaTextMuted,
            style = PrismaTypography.labelSmall,
            fontFamily = FontFamily.Monospace,
        )
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = stage.name,
                color = PrismaTextPrimary,
                style = PrismaTypography.bodyMedium,
                fontWeight = FontWeight.Medium,
            )
            Text(
                text = stage.detail,
                color = PrismaTextMuted,
                style = PrismaTypography.labelSmall,
                fontFamily = FontFamily.Monospace,
            )
        }
        Text(
            text = if (isActive) copy.working else if (isComplete) stage.result else "—",
            modifier = Modifier.padding(end = PrismaSpacing.Lg),
            color = stateColor,
            style = PrismaTypography.labelSmall,
            fontFamily = FontFamily.Monospace,
        )
    }
}

@Composable
private fun CodePane(
    title: String,
    subtitle: String,
    code: String,
) {
    val copy = technicalCopy()
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(PrismaSpacing.Lg)
            .navigationBarsPadding(),
        verticalArrangement = Arrangement.spacedBy(PrismaSpacing.Lg),
    ) {
        Column {
            Text(
                text = title,
                color = PrismaTextPrimary,
                style = PrismaTypography.titleMedium,
                fontWeight = FontWeight.SemiBold,
            )
            Text(
                text = subtitle,
                color = PrismaTextMuted,
                style = PrismaTypography.bodySmall,
            )
        }
        Surface(
            modifier = Modifier
                .fillMaxWidth()
                .weight(1f),
            color = PrismaInspector.Panel,
            shape = RoundedCornerShape(PrismaInspector.Corner),
            border = BorderStroke(PrismaComponents.Border, PrismaBorder),
        ) {
            Text(
                text = code,
                modifier = Modifier
                    .fillMaxSize()
                    .verticalScroll(rememberScrollState())
                    .padding(PrismaSpacing.Lg),
                color = PrismaTextSecondary,
                style = PrismaTypography.bodyMedium,
                fontFamily = FontFamily.Monospace,
            )
        }
        Text(
            text = copy.inspectorSampleNote,
            color = PrismaTextMuted,
            style = PrismaTypography.labelSmall,
        )
    }
}
