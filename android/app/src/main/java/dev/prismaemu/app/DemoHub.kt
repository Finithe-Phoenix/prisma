package dev.prismaemu.app

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay

private enum class DemoKind {
    RealExecution,
    ChatGpt,
    OhMyPosh,
    Notepad,
    Translator,
}

enum class DemoExecutionMode {
    UiSimulation,
    OutputSimulation,
    LivePipeline,
}

data class DemoManifestEntry(
    val id: String,
    val title: String,
    val mode: DemoExecutionMode,
    val windowsPackageExecuted: Boolean,
)

object DemoManifest {
    val entries = listOf(
        DemoManifestEntry("chatgpt-windows", "ChatGPT for Windows", DemoExecutionMode.UiSimulation, false),
        DemoManifestEntry("oh-my-posh", "Oh My Posh", DemoExecutionMode.OutputSimulation, false),
        DemoManifestEntry("notepad-win32", "Notepad · Win32", DemoExecutionMode.UiSimulation, false),
        DemoManifestEntry("prisma-translator", "Prisma Translator", DemoExecutionMode.OutputSimulation, false),
        DemoManifestEntry("real-execution", "Prisma DBT · real probe", DemoExecutionMode.LivePipeline, false),
    )

    fun validate() {
        require(entries.size >= 5)
        require(entries.map { it.id }.toSet().size == entries.size)
        require(entries.single { it.id == "chatgpt-windows" }.windowsPackageExecuted.not())
        require(entries.count { it.mode == DemoExecutionMode.LivePipeline } == 1)
    }
}

data class DemoCopy(
    val demoHub: String,
    val demoHubSubtitle: String,
    val demosAvailable: String,
    val featuredDemo: String,
    val openDemoHub: String,
    val openPreview: String,
    val honestPreview: String,
    val honestPreviewDetail: String,
    val uiSimulation: String,
    val outputSimulation: String,
    val livePipeline: String,
    val packageNotExecuted: String,
    val chatGptDescription: String,
    val chatGreeting: String,
    val chatQuestion: String,
    val chatAnswer: String,
    val compatibilityCheck: String,
    val compatibilityComplete: String,
    val packageRequired: String,
    val bootstrapPlanned: String,
    val winUiBlocked: String,
    val networkPlanned: String,
    val stateNeeded: String,
    val statePlanned: String,
    val stateBlocked: String,
    val ohMyPoshDescription: String,
    val notepadDescription: String,
    val translatorDescription: String,
    val realExecutionDescription: String,
    val interactiveEditor: String,
    val replayOutput: String,
    val typeHere: String,
)

private object DemoCopies {
    private val english = DemoCopy(
        demoHub = "Demo hub",
        demoHubSubtitle = "Compatibility previews · honest execution status",
        demosAvailable = "5 demos available",
        featuredDemo = "Featured Windows target",
        openDemoHub = "Explore demos",
        openPreview = "Open preview",
        honestPreview = "What you are seeing",
        honestPreviewDetail = "UI and output simulations are product prototypes. The real execution probe is the only demo that runs x86-64 guest bytes through the ARM64 JIT; Windows packages remain future compatibility gates.",
        uiSimulation = "Interactive UI simulation",
        outputSimulation = "Deterministic output simulation",
        livePipeline = "Live Prisma pipeline demo",
        packageNotExecuted = "Windows package not executed",
        chatGptDescription = "A focused preview of the ChatGPT for Windows experience and the runtime gaps Prisma must close before the real package can launch.",
        chatGreeting = "How can I help you today?",
        chatQuestion = "Can Prisma run this Windows app on Android?",
        chatAnswer = "This preview demonstrates the intended experience. The real package still needs Win32 bootstrap, WinUI/WebView2 and isolated network/auth support.",
        compatibilityCheck = "Run compatibility check",
        compatibilityComplete = "Compatibility analysis complete",
        packageRequired = "User-provided package",
        bootstrapPlanned = "Win32 bootstrap · planned",
        winUiBlocked = "WinUI / WebView2 · blocked",
        networkPlanned = "Network and auth isolation · planned",
        stateNeeded = "NEEDED",
        statePlanned = "PLANNED",
        stateBlocked = "BLOCKED",
        ohMyPoshDescription = "A deterministic terminal preview for the first real x86-64 Windows CLI compatibility milestone.",
        notepadDescription = "An interactive Win32-style editor preview for windowing, text input and clipboard UX.",
        translatorDescription = "The existing x86 → SSA IR → ARM64 translation trace, driven by live Compose state.",
        realExecutionDescription = "A real PE32+ crosses the Prisma loader and JIT, then executes as ARM64 under the local QEMU worker.",
        interactiveEditor = "Interactive editor preview",
        replayOutput = "Replay output",
        typeHere = "Type in this simulated Win32 editor…",
    )

    private val localized = mapOf(
        "es" to english.copy(
            demoHub = "Centro de demos",
            demoHubSubtitle = "Vistas de compatibilidad · estado real y transparente",
            demosAvailable = "5 demos disponibles",
            featuredDemo = "Objetivo Windows destacado",
            openDemoHub = "Explorar demos",
            openPreview = "Abrir demo",
            honestPreview = "Qué estás viendo",
            honestPreviewDetail = "Las simulaciones de interfaz y salida son prototipos. La prueba de ejecución real es la única demo que pasa bytes guest x86-64 por el JIT ARM64; los paquetes Windows siguen siendo gates futuros.",
            uiSimulation = "Simulación de UI interactiva",
            outputSimulation = "Simulación de salida determinista",
            livePipeline = "Demo del pipeline Prisma",
            packageNotExecuted = "Paquete Windows no ejecutado",
            chatGptDescription = "Una vista enfocada de ChatGPT para Windows y de los componentes que Prisma debe completar antes de arrancar el paquete real.",
            chatGreeting = "¿En qué puedo ayudarte hoy?",
            chatQuestion = "¿Puede Prisma ejecutar esta aplicación de Windows en Android?",
            chatAnswer = "Esta demo enseña la experiencia prevista. El paquete real todavía necesita bootstrap Win32, WinUI/WebView2 y aislamiento de red y autenticación.",
            compatibilityCheck = "Analizar compatibilidad",
            compatibilityComplete = "Análisis de compatibilidad completo",
            packageRequired = "Paquete aportado por el usuario",
            bootstrapPlanned = "Bootstrap Win32 · planificado",
            winUiBlocked = "WinUI / WebView2 · bloqueado",
            networkPlanned = "Red y autenticación aisladas · planificado",
            stateNeeded = "NECESARIO",
            statePlanned = "PLANIFICADO",
            stateBlocked = "BLOQUEADO",
            ohMyPoshDescription = "Vista determinista de terminal para el primer hito real de compatibilidad CLI Windows x86-64.",
            notepadDescription = "Editor interactivo estilo Win32 para validar ventanas, entrada de texto y portapapeles.",
            translatorDescription = "La traza x86 → SSA IR → ARM64 existente, controlada por estado real de Compose.",
            realExecutionDescription = "Un PE32+ real cruza el loader y JIT de Prisma y se ejecuta como ARM64 mediante el worker QEMU local.",
            interactiveEditor = "Editor interactivo",
            replayOutput = "Repetir salida",
            typeHere = "Escribe en este editor Win32 simulado…",
        ),
        "ar" to english.copy(
            demoHub = "مركز العروض",
            demoHubSubtitle = "معاينات التوافق · حالة تنفيذ واضحة",
            demosAvailable = "5 عروض متاحة",
            featuredDemo = "هدف Windows مميز",
            openDemoHub = "استكشاف العروض",
            openPreview = "فتح المعاينة",
            honestPreview = "ما الذي تراه",
            honestPreviewDetail = "محاكاة الواجهة والمخرجات نماذج للمنتج. تتبع المترجم فقط يشغّل منطق Prisma على جهاز x86-64 هذا؛ لا يتم تنفيذ أي حزمة Windows.",
            uiSimulation = "محاكاة واجهة تفاعلية",
            outputSimulation = "محاكاة مخرجات حتمية",
            livePipeline = "عرض مباشر لمسار Prisma",
            packageNotExecuted = "لم يتم تنفيذ حزمة Windows",
            chatGptDescription = "معاينة لتجربة ChatGPT على Windows والفجوات التي يجب على Prisma إغلاقها قبل تشغيل الحزمة الحقيقية.",
            chatGreeting = "كيف يمكنني مساعدتك اليوم؟",
            chatQuestion = "هل يستطيع Prisma تشغيل تطبيق Windows هذا على Android؟",
            chatAnswer = "تعرض هذه المعاينة التجربة المقصودة. ما زالت الحزمة الحقيقية تحتاج إلى Win32 وWinUI/WebView2 وعزل الشبكة والمصادقة.",
            compatibilityCheck = "تحليل التوافق",
            compatibilityComplete = "اكتمل تحليل التوافق",
            packageRequired = "حزمة يقدمها المستخدم",
            bootstrapPlanned = "تهيئة Win32 · مخطط لها",
            winUiBlocked = "WinUI / WebView2 · محظور",
            networkPlanned = "عزل الشبكة والمصادقة · مخطط له",
            stateNeeded = "مطلوب",
            statePlanned = "مخطط",
            stateBlocked = "محظور",
            ohMyPoshDescription = "معاينة طرفية حتمية لأول هدف توافق حقيقي لسطر أوامر Windows x86-64.",
            notepadDescription = "محرر تفاعلي بأسلوب Win32 لاختبار النوافذ وإدخال النص والحافظة.",
            translatorDescription = "تتبع الترجمة الحالي x86 إلى SSA IR إلى ARM64 بحالة Compose حية.",
            realExecutionDescription = "يعبر ملف PE32+ حقيقي محمل Prisma وJIT ثم ينفذ كـ ARM64 عبر عامل QEMU المحلي.",
            interactiveEditor = "معاينة محرر تفاعلي",
            replayOutput = "إعادة الإخراج",
            typeHere = "اكتب في محرر Win32 المحاكى…",
        ),
    )

    fun forTag(tag: String): DemoCopy = localized[tag] ?: english
}

@Composable
fun demoCopy(): DemoCopy = DemoCopies.forTag(LocalPrismaLanguage.current.tag)

@Composable
fun DemoHub(onDismiss: () -> Unit) {
    var selectedDemo by remember { mutableStateOf<DemoKind?>(null) }
    BackHandler {
        if (selectedDemo == null) onDismiss() else selectedDemo = null
    }

    when (selectedDemo) {
        DemoKind.RealExecution -> RealExecutionDemo(onDismiss = { selectedDemo = null })
        DemoKind.ChatGpt -> ChatGptWindowsDemo(onDismiss = { selectedDemo = null })
        DemoKind.OhMyPosh -> OhMyPoshDemo(onDismiss = { selectedDemo = null })
        DemoKind.Notepad -> NotepadWindowsDemo(onDismiss = { selectedDemo = null })
        DemoKind.Translator -> TranslatorDemo(onDismiss = { selectedDemo = null })
        null -> DemoCatalog(
            onDismiss = onDismiss,
            onOpen = { selectedDemo = it },
        )
    }
}

@Composable
private fun DemoCatalog(onDismiss: () -> Unit, onOpen: (DemoKind) -> Unit) {
    val copy = demoCopy()
    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(PrismaBackground)
            .statusBarsPadding(),
    ) {
        PrismaTopBar(
            title = copy.demoHub,
            subtitle = copy.demoHubSubtitle,
            statusColor = PrismaPrimary,
            onBack = onDismiss,
        )
        LazyColumn(
            modifier = Modifier.fillMaxSize(),
            contentPadding = PaddingValues(
                start = PrismaSpacing.Lg,
                top = PrismaSpacing.Lg,
                end = PrismaSpacing.Lg,
                bottom = PrismaSpacing.Section,
            ),
            verticalArrangement = Arrangement.spacedBy(PrismaSpacing.Md),
        ) {
            item {
                Text(
                    text = copy.demosAvailable.uppercase(),
                    color = PrismaPrimary,
                    style = PrismaTypography.labelSmall,
                    fontFamily = FontFamily.Monospace,
                    fontWeight = FontWeight.Bold,
                )
            }
            item {
                DemoCatalogCard(
                    title = "Prisma DBT · real probe",
                    description = copy.realExecutionDescription,
                    fidelity = DemoExecutionMode.LivePipeline,
                    icon = PrismaIconKind.Translate,
                    accent = PrismaSuccess,
                    onClick = { onOpen(DemoKind.RealExecution) },
                )
            }
            item {
                FeaturedChatGptCard(copy = copy, onClick = { onOpen(DemoKind.ChatGpt) })
            }
            item {
                DemoCatalogCard(
                    title = "Oh My Posh",
                    description = copy.ohMyPoshDescription,
                    fidelity = DemoExecutionMode.OutputSimulation,
                    icon = PrismaIconKind.Terminal,
                    accent = PrismaAccent,
                    onClick = { onOpen(DemoKind.OhMyPosh) },
                )
            }
            item {
                DemoCatalogCard(
                    title = "Notepad · Win32",
                    description = copy.notepadDescription,
                    fidelity = DemoExecutionMode.UiSimulation,
                    icon = PrismaIconKind.Library,
                    accent = PrismaWarning,
                    onClick = { onOpen(DemoKind.Notepad) },
                )
            }
            item {
                DemoCatalogCard(
                    title = "Prisma Translator",
                    description = copy.translatorDescription,
                    fidelity = DemoExecutionMode.OutputSimulation,
                    icon = PrismaIconKind.Translate,
                    accent = PrismaPrimary,
                    onClick = { onOpen(DemoKind.Translator) },
                )
            }
            item {
                HonestPreviewNotice(copy)
            }
        }
    }
}

@Composable
fun DemoSpotlightCard(onClick: () -> Unit) {
    val copy = demoCopy()
    Surface(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = PrismaSpacing.Lg, vertical = PrismaSpacing.Sm)
            .clickable(onClick = onClick),
        color = PrismaSurface,
        shape = RoundedCornerShape(PrismaRadii.Xl),
        border = BorderStroke(PrismaComponents.Border, PrismaSecondary.copy(alpha = 0.45f)),
    ) {
        Row(
            modifier = Modifier.padding(PrismaSpacing.Xl),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            ChatGptMark(
                modifier = Modifier
                    .size(52.dp)
                    .background(PrismaWhite, RoundedCornerShape(PrismaRadii.Lg))
                    .padding(PrismaSpacing.Md),
                tint = PrismaInk1000,
            )
            Column(
                modifier = Modifier
                    .weight(1f)
                    .padding(horizontal = PrismaSpacing.Lg),
            ) {
                Text(
                    text = copy.featuredDemo.uppercase(),
                    color = PrismaSecondary,
                    style = PrismaTypography.labelSmall,
                    fontFamily = FontFamily.Monospace,
                )
                Text(
                    text = "ChatGPT for Windows",
                    color = PrismaTextPrimary,
                    style = PrismaTypography.titleLarge,
                    fontWeight = FontWeight.Bold,
                )
                Text(
                    text = copy.demosAvailable,
                    color = PrismaTextMuted,
                    style = PrismaTypography.bodySmall,
                )
            }
            Text(
                text = copy.openDemoHub,
                color = PrismaPrimary,
                style = PrismaTypography.labelMedium,
                fontWeight = FontWeight.Bold,
            )
        }
    }
}

@Composable
private fun FeaturedChatGptCard(copy: DemoCopy, onClick: () -> Unit) {
    Surface(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick),
        color = PrismaSurfaceElevated,
        shape = RoundedCornerShape(PrismaRadii.Xl),
        border = BorderStroke(PrismaComponents.Border, PrismaSecondary.copy(alpha = 0.5f)),
    ) {
        Column(modifier = Modifier.padding(PrismaSpacing.Xl)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                ChatGptMark(
                    modifier = Modifier
                        .size(52.dp)
                        .background(PrismaWhite, RoundedCornerShape(PrismaRadii.Lg))
                        .padding(PrismaSpacing.Md),
                    tint = PrismaInk1000,
                )
                Column(
                    modifier = Modifier
                        .weight(1f)
                        .padding(start = PrismaSpacing.Lg),
                ) {
                    Text(copy.featuredDemo.uppercase(), color = PrismaSecondary, style = PrismaTypography.labelSmall)
                    Text("ChatGPT for Windows", color = PrismaTextPrimary, style = PrismaTypography.headlineSmall)
                }
            }
            Text(
                text = copy.chatGptDescription,
                modifier = Modifier.padding(top = PrismaSpacing.Lg),
                color = PrismaTextSecondary,
                style = PrismaTypography.bodyMedium,
            )
            Box(modifier = Modifier.padding(top = PrismaSpacing.Md)) {
                DemoBadge(DemoExecutionMode.UiSimulation)
            }
            Text(
                text = copy.openPreview,
                modifier = Modifier.padding(top = PrismaSpacing.Lg),
                color = PrismaPrimary,
                style = PrismaTypography.labelMedium,
                fontWeight = FontWeight.Bold,
            )
        }
    }
}

@Composable
private fun DemoCatalogCard(
    title: String,
    description: String,
    fidelity: DemoExecutionMode,
    icon: PrismaIconKind,
    accent: Color,
    onClick: () -> Unit,
) {
    Surface(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick),
        color = PrismaSurface,
        shape = RoundedCornerShape(PrismaRadii.Lg),
        border = BorderStroke(PrismaComponents.Border, PrismaBorderSubtle),
    ) {
        Column(modifier = Modifier.padding(PrismaSpacing.Lg)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Box(
                    modifier = Modifier
                        .size(48.dp)
                        .background(accent.copy(alpha = 0.12f), RoundedCornerShape(PrismaRadii.Md)),
                    contentAlignment = Alignment.Center,
                ) {
                    PrismaGlyph(icon, accent, Modifier.size(22.dp))
                }
                Column(
                    modifier = Modifier
                        .weight(1f)
                        .padding(start = PrismaSpacing.Lg),
                ) {
                    Text(title, color = PrismaTextPrimary, style = PrismaTypography.titleMedium)
                    Text(
                        text = description,
                        modifier = Modifier.padding(top = PrismaSpacing.Xs),
                        color = PrismaTextMuted,
                        style = PrismaTypography.bodySmall,
                        maxLines = 3,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
            Box(modifier = Modifier.padding(top = PrismaSpacing.Md)) {
                DemoBadge(fidelity)
            }
        }
    }
}

@Composable
private fun DemoBadge(fidelity: DemoExecutionMode) {
    val copy = demoCopy()
    val (label, color) = when (fidelity) {
        DemoExecutionMode.UiSimulation -> copy.uiSimulation to PrismaSecondary
        DemoExecutionMode.OutputSimulation -> copy.outputSimulation to PrismaAccent
        DemoExecutionMode.LivePipeline -> copy.livePipeline to PrismaSuccess
    }
    Surface(
        color = color.copy(alpha = 0.11f),
        shape = RoundedCornerShape(PrismaRadii.Pill),
        border = BorderStroke(PrismaComponents.Border, color.copy(alpha = 0.4f)),
    ) {
        Text(
            text = label,
            modifier = Modifier.padding(horizontal = PrismaSpacing.Sm, vertical = PrismaSpacing.Xs),
            color = color,
            style = PrismaTypography.labelSmall,
            maxLines = 2,
        )
    }
}

@Composable
private fun HonestPreviewNotice(copy: DemoCopy) {
    Surface(
        color = PrismaWarning.copy(alpha = 0.08f),
        shape = RoundedCornerShape(PrismaRadii.Lg),
        border = BorderStroke(PrismaComponents.Border, PrismaWarning.copy(alpha = 0.32f)),
    ) {
        Row(modifier = Modifier.padding(PrismaSpacing.Lg), verticalAlignment = Alignment.Top) {
            Box(
                modifier = Modifier
                    .padding(top = PrismaSpacing.Xs)
                    .size(8.dp)
                    .background(PrismaWarning, CircleShape),
            )
            Column(modifier = Modifier.padding(start = PrismaSpacing.Md)) {
                Text(copy.honestPreview, color = PrismaTextPrimary, style = PrismaTypography.titleMedium)
                Text(
                    text = copy.honestPreviewDetail,
                    modifier = Modifier.padding(top = PrismaSpacing.Xs),
                    color = PrismaTextSecondary,
                    style = PrismaTypography.bodySmall,
                )
            }
        }
    }
}

@Composable
private fun ChatGptWindowsDemo(onDismiss: () -> Unit) {
    val copy = demoCopy()
    var analyzed by remember { mutableStateOf(false) }
    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(PrismaBackground)
            .statusBarsPadding(),
    ) {
        PrismaTopBar(
            title = "ChatGPT for Windows",
            subtitle = copy.packageNotExecuted,
            statusColor = PrismaWarning,
            onBack = onDismiss,
        )
        LazyColumn(
            modifier = Modifier.fillMaxSize(),
            contentPadding = PaddingValues(PrismaSpacing.Lg),
            verticalArrangement = Arrangement.spacedBy(PrismaSpacing.Lg),
        ) {
            item { ChatGptWindow(copy) }
            item {
                Button(
                    onClick = { analyzed = true },
                    modifier = Modifier
                        .fillMaxWidth()
                        .height(PrismaComponents.ActionHeight),
                    colors = ButtonDefaults.buttonColors(
                        containerColor = PrismaPrimary,
                        contentColor = PrismaInk1000,
                    ),
                    shape = RoundedCornerShape(PrismaRadii.Md),
                ) {
                    Text(if (analyzed) copy.compatibilityComplete else copy.compatibilityCheck)
                }
            }
            if (analyzed) {
                item {
                    CompatibilityChecklist(copy)
                }
            }
            item { HonestPreviewNotice(copy) }
        }
    }
}

@Composable
private fun ChatGptWindow(copy: DemoCopy) {
    Surface(
        color = PrismaWhite,
        shape = RoundedCornerShape(PrismaRadii.Lg),
        border = BorderStroke(PrismaComponents.Border, PrismaBorder),
    ) {
        Column(modifier = Modifier.height(430.dp)) {
            WindowChrome(title = "ChatGPT")
            Row(modifier = Modifier.fillMaxSize()) {
                Column(
                    modifier = Modifier
                        .width(74.dp)
                        .fillMaxHeight()
                        .background(PrismaInk900)
                        .padding(PrismaSpacing.Md),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    ChatGptMark(Modifier.size(28.dp), PrismaWhite)
                    Spacer(Modifier.height(PrismaSpacing.Xl))
                    repeat(3) { index ->
                        Box(
                            modifier = Modifier
                                .padding(vertical = PrismaSpacing.Sm)
                                .size(30.dp)
                                .background(
                                    if (index == 0) PrismaPrimary.copy(alpha = 0.18f) else PrismaInk800,
                                    RoundedCornerShape(PrismaRadii.Sm),
                                ),
                        )
                    }
                }
                Column(
                    modifier = Modifier
                        .weight(1f)
                        .fillMaxHeight()
                        .background(PrismaWhite)
                        .padding(PrismaSpacing.Lg),
                ) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        ChatGptMark(Modifier.size(30.dp), PrismaInk1000)
                        Text(
                            text = copy.chatGreeting,
                            modifier = Modifier.padding(start = PrismaSpacing.Md),
                            color = PrismaInk1000,
                            style = PrismaTypography.titleMedium,
                        )
                    }
                    ChatBubble(
                        text = copy.chatQuestion,
                        isUser = true,
                        modifier = Modifier.padding(top = PrismaSpacing.Xl),
                    )
                    ChatBubble(
                        text = copy.chatAnswer,
                        isUser = false,
                        modifier = Modifier.padding(top = PrismaSpacing.Md),
                    )
                    Spacer(Modifier.weight(1f))
                    Surface(
                        color = PrismaWhite,
                        shape = RoundedCornerShape(PrismaRadii.Md),
                        border = BorderStroke(PrismaComponents.Border, PrismaSlate300),
                    ) {
                        Text(
                            text = "Message ChatGPT",
                            modifier = Modifier.padding(PrismaSpacing.Md),
                            color = PrismaSlate500,
                            style = PrismaTypography.bodySmall,
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun ChatBubble(text: String, isUser: Boolean, modifier: Modifier = Modifier) {
    Row(modifier = modifier.fillMaxWidth()) {
        if (isUser) Spacer(Modifier.width(42.dp))
        Surface(
            modifier = Modifier.weight(1f),
            color = if (isUser) PrismaInk850 else PrismaWhite,
            shape = RoundedCornerShape(PrismaRadii.Md),
            border = if (isUser) null else BorderStroke(PrismaComponents.Border, PrismaInk800),
        ) {
            Text(
                text = text,
                modifier = Modifier.padding(PrismaSpacing.Md),
                color = if (isUser) PrismaWhite else PrismaInk1000,
                style = PrismaTypography.bodySmall,
            )
        }
        if (!isUser) Spacer(Modifier.width(28.dp))
    }
}

@Composable
private fun CompatibilityChecklist(copy: DemoCopy) {
    Surface(
        color = PrismaSurface,
        shape = RoundedCornerShape(PrismaRadii.Lg),
        border = BorderStroke(PrismaComponents.Border, PrismaBorderSubtle),
    ) {
        Column {
            CompatibilityRow(copy.packageRequired, copy.stateNeeded, PrismaWarning)
            HorizontalDivider(color = PrismaBorderSubtle)
            CompatibilityRow(copy.bootstrapPlanned, copy.statePlanned, PrismaSecondary)
            HorizontalDivider(color = PrismaBorderSubtle)
            CompatibilityRow(copy.winUiBlocked, copy.stateBlocked, PrismaError)
            HorizontalDivider(color = PrismaBorderSubtle)
            CompatibilityRow(copy.networkPlanned, copy.statePlanned, PrismaSecondary)
        }
    }
}

@Composable
private fun CompatibilityRow(label: String, state: String, color: Color) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(PrismaSpacing.Lg),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(8.dp).background(color, CircleShape))
        Text(
            text = label,
            modifier = Modifier
                .weight(1f)
                .padding(horizontal = PrismaSpacing.Md),
            color = PrismaTextSecondary,
            style = PrismaTypography.bodySmall,
        )
        Text(state, color = color, style = PrismaTypography.labelSmall, fontFamily = FontFamily.Monospace)
    }
}

@Composable
private fun OhMyPoshDemo(onDismiss: () -> Unit) {
    val copy = demoCopy()
    var runKey by remember { mutableIntStateOf(0) }
    var visibleLines by remember { mutableIntStateOf(1) }
    val lines = listOf(
        "> oh-my-posh.exe version",
        "26.17.1",
        "> oh-my-posh.exe print primary --config sample.omp.json --shell uni",
        "[ PRISMA ]  ~/guest/c/Users/Danny  [ main +2 ]",
        "> _",
    )
    LaunchedEffect(runKey) {
        visibleLines = 1
        while (visibleLines < lines.size) {
            delay(320)
            visibleLines += 1
        }
    }
    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(PrismaBackground)
            .statusBarsPadding(),
    ) {
        PrismaTopBar(
            title = "Oh My Posh",
            subtitle = copy.packageNotExecuted,
            statusColor = PrismaAccent,
            onBack = onDismiss,
        )
        Column(
            modifier = Modifier
                .fillMaxSize()
                .navigationBarsPadding()
                .padding(PrismaSpacing.Lg),
            verticalArrangement = Arrangement.spacedBy(PrismaSpacing.Lg),
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                DemoBadge(DemoExecutionMode.OutputSimulation)
                Spacer(Modifier.weight(1f))
                TextButton(onClick = { runKey += 1 }) {
                    Text(copy.replayOutput, color = PrismaPrimary)
                }
            }
            Surface(
                modifier = Modifier
                    .fillMaxWidth()
                    .weight(1f),
                color = PrismaInspector.Panel,
                shape = RoundedCornerShape(PrismaRadii.Lg),
                border = BorderStroke(PrismaComponents.Border, PrismaBorder),
            ) {
                Column(modifier = Modifier.padding(PrismaSpacing.Xl)) {
                    Text("PRISMA TERMINAL  /  x86-64 GUEST", color = PrismaTextMuted, style = PrismaTypography.labelSmall, fontFamily = FontFamily.Monospace)
                    lines.take(visibleLines).forEachIndexed { index, line ->
                        Text(
                            text = line,
                            modifier = Modifier.padding(top = if (index == 0) PrismaSpacing.Xl else PrismaSpacing.Md),
                            color = when (index) {
                                1 -> PrismaSuccess
                                3 -> PrismaAccent
                                else -> PrismaTextPrimary
                            },
                            style = PrismaTypography.bodyMedium,
                            fontFamily = FontFamily.Monospace,
                        )
                    }
                }
            }
            HonestPreviewNotice(copy)
        }
    }
}

@Composable
private fun NotepadWindowsDemo(onDismiss: () -> Unit) {
    val copy = demoCopy()
    var note by remember { mutableStateOf("") }
    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(PrismaBackground)
            .statusBarsPadding(),
    ) {
        PrismaTopBar(
            title = "Notepad · Win32",
            subtitle = copy.interactiveEditor,
            statusColor = PrismaWarning,
            onBack = onDismiss,
        )
        Column(
            modifier = Modifier
                .fillMaxSize()
                .imePadding()
                .navigationBarsPadding()
                .padding(PrismaSpacing.Lg),
            verticalArrangement = Arrangement.spacedBy(PrismaSpacing.Lg),
        ) {
            DemoBadge(DemoExecutionMode.UiSimulation)
            Surface(
                modifier = Modifier
                    .fillMaxWidth()
                    .weight(1f),
                color = PrismaWhite,
                shape = RoundedCornerShape(PrismaRadii.Lg),
                border = BorderStroke(PrismaComponents.Border, PrismaBorder),
            ) {
                Column {
                    WindowChrome("Untitled - Notepad")
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .background(PrismaWhite)
                            .padding(horizontal = PrismaSpacing.Md, vertical = PrismaSpacing.Sm),
                        horizontalArrangement = Arrangement.spacedBy(PrismaSpacing.Xl),
                    ) {
                        listOf("File", "Edit", "Format", "View", "Help").forEach {
                            Text(it, color = PrismaInk1000, style = PrismaTypography.labelSmall)
                        }
                    }
                    HorizontalDivider(color = PrismaSlate300)
                    BasicTextField(
                        value = note,
                        onValueChange = { note = it },
                        modifier = Modifier
                            .fillMaxSize()
                            .padding(PrismaSpacing.Lg),
                        textStyle = TextStyle(
                            color = PrismaInk1000,
                            fontFamily = FontFamily.Monospace,
                            fontSize = PrismaTypography.bodyMedium.fontSize,
                        ),
                        decorationBox = { inner ->
                            if (note.isEmpty()) {
                                Text(copy.typeHere, color = PrismaSlate500, style = PrismaTypography.bodyMedium)
                            }
                            inner()
                        },
                    )
                }
            }
            HonestPreviewNotice(copy)
        }
    }
}

@Composable
private fun WindowChrome(title: String) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .height(42.dp)
            .background(PrismaInk850)
            .padding(horizontal = PrismaSpacing.Md),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(title, modifier = Modifier.weight(1f), color = PrismaWhite, style = PrismaTypography.labelMedium)
        listOf(PrismaSlate500, PrismaWarning, PrismaError).forEach { color ->
            Box(
                modifier = Modifier
                    .padding(start = PrismaSpacing.Md)
                    .size(10.dp)
                    .background(color, CircleShape),
            )
        }
    }
}

@Composable
private fun ChatGptMark(modifier: Modifier = Modifier, tint: Color) {
    val description = "ChatGPT"
    Canvas(modifier = modifier.semantics { contentDescription = description }) {
        val radius = size.minDimension * 0.28f
        repeat(6) { index ->
            val angle = index * kotlin.math.PI.toFloat() / 3f
            val next = (index + 2) * kotlin.math.PI.toFloat() / 3f
            drawLine(
                color = tint,
                start = Offset(
                    center.x + kotlin.math.cos(angle) * radius,
                    center.y + kotlin.math.sin(angle) * radius,
                ),
                end = Offset(
                    center.x + kotlin.math.cos(next) * radius,
                    center.y + kotlin.math.sin(next) * radius,
                ),
                strokeWidth = 1.8.dp.toPx(),
                cap = StrokeCap.Round,
            )
        }
        drawCircle(tint, radius = radius * 0.42f, center = center, style = Stroke(1.8.dp.toPx()))
    }
}
