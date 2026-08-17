package dev.prismaemu.app

import android.net.Uri
import android.widget.Toast
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import java.io.FileOutputStream

private enum class MainDestination(val icon: PrismaIconKind, val label: UiText) {
    Home(PrismaIconKind.Home, UiText.Home),
    Library(PrismaIconKind.Library, UiText.Library),
    Activity(PrismaIconKind.Activity, UiText.Activity),
    Settings(PrismaIconKind.Settings, UiText.Settings),
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PrismaAppShell(
    store: ContainerStore,
    onRunExe: (String) -> Unit,
    languageTag: String,
    onLanguageChange: (String) -> Unit,
) {
    val technical = technicalCopy()
    val state by store.state.collectAsState()
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var destination by remember { mutableStateOf(MainDestination.Home) }
    var showCreateDialog by remember { mutableStateOf(false) }
    var showSteamImporter by remember { mutableStateOf(false) }
    var showGamepadMapper by remember { mutableStateOf(false) }
    var showTerminal by remember { mutableStateOf(false) }
    var showTranslatorDemo by remember { mutableStateOf(false) }
    var showDemoHub by remember { mutableStateOf(false) }
    var showLanguagePicker by remember { mutableStateOf(false) }

    Box(modifier = Modifier.fillMaxSize()) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .background(PrismaBackground)
                .statusBarsPadding(),
        ) {
            BrandHeader(engineReady = false)
            Box(modifier = Modifier.weight(1f)) {
                when (destination) {
                    MainDestination.Home -> HomeScreen(
                        containers = state.containers,
                        onImport = {
                            val firstContainer = state.containers.firstOrNull()
                            if (firstContainer == null) {
                                showCreateDialog = true
                            } else {
                                store.selectContainer(firstContainer.id)
                            }
                        },
                        onNewWorkspace = { showCreateDialog = true },
                        onViewAll = { destination = MainDestination.Library },
                        onContainer = { store.selectContainer(it) },
                        onInspector = { showTranslatorDemo = true },
                        onDemos = { showDemoHub = true },
                        onTerminal = { showTerminal = true },
                        onSteam = { showSteamImporter = true },
                        onInput = { showGamepadMapper = true },
                    )
                    MainDestination.Library -> LibraryScreen(
                        containers = state.containers,
                        onNewWorkspace = { showCreateDialog = true },
                        onContainer = { store.selectContainer(it) },
                    )
                    MainDestination.Activity -> ActivityScreen()
                    MainDestination.Settings -> SettingsScreen(
                        languageTag = languageTag,
                        onLanguage = { showLanguagePicker = true },
                    )
                }
            }
            PrismaBottomBar(selected = destination, onSelect = { destination = it })
        }
    }

    if (showCreateDialog) {
        CreateContainerDialog(
            onDismiss = { showCreateDialog = false },
            onCreate = { name ->
                store.createContainer(name)
                showCreateDialog = false
            },
        )
    }

    if (showSteamImporter) {
        PrototypeDialog(
            title = technical.steamLibrary,
            description = technical.libraryDiscovery,
            onDismiss = { showSteamImporter = false },
        )
    }

    if (showGamepadMapper) {
        PrototypeDialog(
            title = technical.inputMapper,
            description = technical.touchBindings,
            onDismiss = { showGamepadMapper = false },
        )
    }

    if (showTerminal) TerminalView(onDismiss = { showTerminal = false })
    if (showTranslatorDemo) TranslatorDemo(onDismiss = { showTranslatorDemo = false })
    if (showDemoHub) DemoHub(onDismiss = { showDemoHub = false })
    if (showLanguagePicker) {
        LanguagePicker(
            selectedTag = languageTag,
            onDismiss = { showLanguagePicker = false },
            onSelect = { tag ->
                onLanguageChange(tag)
                showLanguagePicker = false
            },
        )
    }

    state.selectedContainerId?.let { selectedId ->
        state.containers.find { it.id == selectedId }?.let { selected ->
            ContainerActionSheet(
                container = selected,
                onDismiss = { store.selectContainer(null) },
                onInstall = { uri ->
                    store.installExe(selected.id, uri.toString())
                    store.selectContainer(null)
                    scope.launch(Dispatchers.IO) {
                        importExecutable(context, selected, uri, technical, onRunExe)
                    }
                },
            )
        }
    }
}

@Composable
private fun BrandHeader(engineReady: Boolean) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .height(64.dp)
            .padding(horizontal = PrismaSpacing.Lg),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        PrismaMark()
        Text(
            text = "PRISMA",
            modifier = Modifier.padding(start = PrismaSpacing.Md),
            color = PrismaTextPrimary,
            style = PrismaTypography.titleMedium,
            fontWeight = FontWeight.Black,
            letterSpacing = androidx.compose.ui.unit.TextUnit.Unspecified,
        )
        Spacer(modifier = Modifier.weight(1f))
        Surface(
            color = if (engineReady) {
                PrismaSuccess.copy(alpha = 0.12f)
            } else {
                PrismaWarning.copy(alpha = 0.12f)
            },
            shape = RoundedCornerShape(PrismaRadii.Pill),
            border = BorderStroke(
                PrismaComponents.Border,
                if (engineReady) PrismaSuccess.copy(alpha = 0.35f) else PrismaWarning.copy(alpha = 0.35f),
            ),
        ) {
            Row(
                modifier = Modifier.padding(horizontal = PrismaSpacing.Md, vertical = PrismaSpacing.Sm),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Box(
                    modifier = Modifier
                        .size(PrismaComponents.StatusDot)
                        .background(if (engineReady) PrismaSuccess else PrismaWarning, CircleShape),
                )
                Text(
                    text = if (engineReady) tr(UiText.EngineReady) else tr(UiText.PreviewMode),
                    modifier = Modifier.padding(start = PrismaSpacing.Sm),
                    color = if (engineReady) PrismaSuccess else PrismaWarning,
                    style = PrismaTypography.labelSmall,
                    fontWeight = FontWeight.Bold,
                )
            }
        }
    }
}

@Composable
private fun HomeScreen(
    containers: List<Container>,
    onImport: () -> Unit,
    onNewWorkspace: () -> Unit,
    onViewAll: () -> Unit,
    onContainer: (String) -> Unit,
    onInspector: () -> Unit,
    onDemos: () -> Unit,
    onTerminal: () -> Unit,
    onSteam: () -> Unit,
    onInput: () -> Unit,
) {
    val technical = technicalCopy()
    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(bottom = PrismaSpacing.Xxl),
    ) {
        item {
            HeroCard(onImport = onImport, onTryTranslation = onInspector)
        }
        item {
            DemoSpotlightCard(onClick = onDemos)
        }
        item {
            PrismaSectionHeader(
                title = tr(UiText.RecentWorkspaces),
                detail = technical.winePrefixesDetail,
                actionLabel = tr(UiText.ViewAll),
                onAction = onViewAll,
            )
        }
        items(containers.take(2), key = { it.id }) { container ->
            WorkspaceCard(container = container, onClick = { onContainer(container.id) })
        }
        item {
            TextButton(
                onClick = onNewWorkspace,
                modifier = Modifier.padding(horizontal = PrismaSpacing.Sm),
            ) {
                Icon(Icons.Default.Add, contentDescription = null, tint = PrismaPrimary)
                Text(
                    text = tr(UiText.NewWorkspace),
                    modifier = Modifier.padding(start = PrismaSpacing.Sm),
                    color = PrismaPrimary,
                )
            }
        }
        item { PrismaSectionHeader(title = tr(UiText.Tools)) }
        item {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = PrismaSpacing.Lg),
                horizontalArrangement = Arrangement.spacedBy(PrismaSpacing.Md),
            ) {
                ToolCard(
                    icon = PrismaIconKind.Translate,
                    title = technical.translationInspector,
                    detail = "x86 → IR → ARM64",
                    accent = PrismaPrimary,
                    onClick = onInspector,
                    modifier = Modifier.weight(1f),
                )
                ToolCard(
                    icon = PrismaIconKind.Terminal,
                    title = technical.developerTerminal,
                    detail = technical.guestProcessIo,
                    accent = PrismaSecondary,
                    onClick = onTerminal,
                    modifier = Modifier.weight(1f),
                )
            }
        }
        item {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = PrismaSpacing.Lg, vertical = PrismaSpacing.Md),
                horizontalArrangement = Arrangement.spacedBy(PrismaSpacing.Md),
            ) {
                ToolCard(
                    icon = PrismaIconKind.Library,
                    title = technical.steamLibrary,
                    detail = tr(UiText.ComingSoon),
                    accent = PrismaAccent,
                    onClick = onSteam,
                    modifier = Modifier.weight(1f),
                )
                ToolCard(
                    icon = PrismaIconKind.Controller,
                    title = technical.inputMapper,
                    detail = tr(UiText.ComingSoon),
                    accent = PrismaWarning,
                    onClick = onInput,
                    modifier = Modifier.weight(1f),
                )
            }
        }
    }
}

@Composable
private fun HeroCard(onImport: () -> Unit, onTryTranslation: () -> Unit) {
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = PrismaSpacing.Lg, vertical = PrismaSpacing.Sm)
            .height(PrismaComponents.HeroHeight)
            .clip(RoundedCornerShape(PrismaRadii.Hero))
            .background(
                Brush.linearGradient(
                    listOf(Color(0xFF13282B), Color(0xFF17172A), PrismaSurface),
                ),
            ),
    ) {
        Canvas(modifier = Modifier.fillMaxSize()) {
            drawCircle(
                color = PrismaPrimary.copy(alpha = 0.08f),
                radius = size.width * 0.52f,
                center = androidx.compose.ui.geometry.Offset(size.width * 0.92f, size.height * 0.08f),
            )
            drawCircle(
                color = PrismaSecondary.copy(alpha = 0.08f),
                radius = size.width * 0.38f,
                center = androidx.compose.ui.geometry.Offset(size.width * 0.78f, size.height * 0.9f),
            )
            drawLine(
                color = PrismaPrimary.copy(alpha = 0.2f),
                start = androidx.compose.ui.geometry.Offset(size.width * 0.72f, 0f),
                end = androidx.compose.ui.geometry.Offset(size.width, size.height * 0.38f),
                strokeWidth = 1.dp.toPx(),
                cap = StrokeCap.Round,
            )
        }
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(PrismaSpacing.Xxl),
        ) {
            Text(
                text = "X86-64  /  ARM64",
                color = PrismaPrimary,
                style = PrismaTypography.labelMedium,
                fontFamily = FontFamily.Monospace,
                fontWeight = FontWeight.Bold,
            )
            Text(
                text = tr(UiText.HeroTitle),
                modifier = Modifier.padding(top = PrismaSpacing.Md),
                color = PrismaTextPrimary,
                style = PrismaTypography.displaySmall,
                fontWeight = FontWeight.Black,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                text = tr(UiText.HeroSubtitle),
                modifier = Modifier.padding(top = PrismaSpacing.Sm),
                color = PrismaTextSecondary,
                style = PrismaTypography.bodyLarge,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            Spacer(modifier = Modifier.weight(1f))
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(PrismaSpacing.Md),
            ) {
                Button(
                    onClick = onImport,
                    modifier = Modifier
                        .weight(1f)
                        .height(PrismaComponents.ActionHeight),
                    shape = RoundedCornerShape(PrismaRadii.Md),
                    colors = ButtonDefaults.buttonColors(
                        containerColor = PrismaPrimary,
                        contentColor = PrismaInk1000,
                    ),
                ) {
                    PrismaGlyph(PrismaIconKind.Import, PrismaInk1000, Modifier.size(20.dp))
                    Text(
                        text = tr(UiText.ImportApp),
                        modifier = Modifier.padding(start = PrismaSpacing.Sm),
                        style = PrismaTypography.labelMedium,
                        textAlign = TextAlign.Center,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                OutlinedButton(
                    onClick = onTryTranslation,
                    modifier = Modifier
                        .weight(1f)
                        .height(PrismaComponents.ActionHeight),
                    shape = RoundedCornerShape(PrismaRadii.Md),
                    border = BorderStroke(PrismaComponents.Border, PrismaBorder),
                ) {
                    Text(
                        text = tr(UiText.TryTranslation),
                        color = PrismaTextPrimary,
                        style = PrismaTypography.labelMedium,
                        textAlign = TextAlign.Center,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
        }
    }
}

@Composable
private fun WorkspaceCard(container: Container, onClick: () -> Unit) {
    Surface(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = PrismaSpacing.Lg, vertical = PrismaSpacing.Xs)
            .clickable(onClick = onClick),
        color = PrismaSurface,
        shape = RoundedCornerShape(PrismaRadii.Lg),
        border = BorderStroke(PrismaComponents.Border, PrismaBorderSubtle),
    ) {
        Row(
            modifier = Modifier.padding(PrismaSpacing.Lg),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(
                modifier = Modifier
                    .size(50.dp)
                    .background(
                        if (container.isRunning) {
                            PrismaSuccess.copy(alpha = 0.12f)
                        } else {
                            PrismaPrimary.copy(alpha = 0.1f)
                        },
                        RoundedCornerShape(PrismaRadii.Md),
                    ),
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    text = container.name.take(1).uppercase(),
                    color = if (container.isRunning) PrismaSuccess else PrismaPrimary,
                    style = PrismaTypography.titleLarge,
                    fontWeight = FontWeight.Black,
                )
            }
            Column(
                modifier = Modifier
                    .weight(1f)
                    .padding(horizontal = PrismaSpacing.Lg),
            ) {
                Text(
                    text = container.name,
                    color = PrismaTextPrimary,
                    style = PrismaTypography.titleMedium,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    text = "${container.installedGames} ${tr(UiText.Apps)} · ${container.winePrefix.substringAfterLast('/')}",
                    color = PrismaTextMuted,
                    style = PrismaTypography.bodySmall,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            PrismaGlyph(PrismaIconKind.Chevron, PrismaTextMuted, Modifier.size(18.dp))
        }
    }
}

@Composable
private fun ToolCard(
    icon: PrismaIconKind,
    title: String,
    detail: String,
    accent: Color,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Surface(
        modifier = modifier
            .height(142.dp)
            .clickable(onClick = onClick),
        color = PrismaSurface,
        shape = RoundedCornerShape(PrismaRadii.Lg),
        border = BorderStroke(PrismaComponents.Border, PrismaBorderSubtle),
    ) {
        Column(modifier = Modifier.padding(PrismaSpacing.Lg)) {
            Box(
                modifier = Modifier
                    .size(40.dp)
                    .background(accent.copy(alpha = 0.11f), RoundedCornerShape(PrismaRadii.Md)),
                contentAlignment = Alignment.Center,
            ) {
                PrismaGlyph(icon, accent, Modifier.size(22.dp))
            }
            Spacer(modifier = Modifier.weight(1f))
            Text(
                text = title,
                color = PrismaTextPrimary,
                style = PrismaTypography.titleMedium,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                text = detail,
                color = PrismaTextMuted,
                style = PrismaTypography.labelSmall,
                fontFamily = FontFamily.Monospace,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

@Composable
private fun LibraryScreen(
    containers: List<Container>,
    onNewWorkspace: () -> Unit,
    onContainer: (String) -> Unit,
) {
    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(bottom = PrismaSpacing.Xxl),
    ) {
        item {
            ScreenIntroduction(
                title = tr(UiText.Library),
                description = tr(UiText.LibraryDescription),
            )
        }
        item {
            Button(
                onClick = onNewWorkspace,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = PrismaSpacing.Lg)
                    .height(PrismaComponents.ActionHeight),
                shape = RoundedCornerShape(PrismaRadii.Md),
                colors = ButtonDefaults.buttonColors(
                    containerColor = PrismaPrimary,
                    contentColor = PrismaInk1000,
                ),
            ) {
                Icon(Icons.Default.Add, contentDescription = null)
                Text(tr(UiText.NewWorkspace), Modifier.padding(start = PrismaSpacing.Sm))
            }
        }
        item { Spacer(modifier = Modifier.height(PrismaSpacing.Md)) }
        items(containers, key = { it.id }) { container ->
            WorkspaceCard(container = container, onClick = { onContainer(container.id) })
        }
    }
}

@Composable
private fun ActivityScreen() {
    val technical = technicalCopy()
    Column(modifier = Modifier.fillMaxSize()) {
        ScreenIntroduction(
            title = tr(UiText.Activity),
            description = tr(UiText.ActivityDescription),
        )
        Surface(
            modifier = Modifier
                .fillMaxWidth()
                .padding(PrismaSpacing.Lg)
                .weight(1f),
            color = PrismaSurface,
            shape = RoundedCornerShape(PrismaRadii.Xl),
            border = BorderStroke(PrismaComponents.Border, PrismaBorderSubtle),
        ) {
            Column(
                modifier = Modifier.fillMaxSize().padding(PrismaSpacing.Xxl),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.Center,
            ) {
                Box(
                    modifier = Modifier
                        .size(72.dp)
                        .background(PrismaPrimary.copy(alpha = 0.09f), CircleShape),
                    contentAlignment = Alignment.Center,
                ) {
                    PrismaGlyph(PrismaIconKind.Activity, PrismaPrimary, Modifier.size(32.dp))
                }
                Text(
                    text = tr(UiText.NoActivity),
                    modifier = Modifier.padding(top = PrismaSpacing.Xl),
                    color = PrismaTextPrimary,
                    style = PrismaTypography.titleLarge,
                    textAlign = TextAlign.Center,
                )
                Text(
                    text = technical.activityEmptyDetail,
                    modifier = Modifier.padding(top = PrismaSpacing.Sm),
                    color = PrismaTextMuted,
                    style = PrismaTypography.bodyMedium,
                    textAlign = TextAlign.Center,
                )
            }
        }
    }
}

@Composable
private fun SettingsScreen(languageTag: String, onLanguage: () -> Unit) {
    val language = PrismaLanguages.language(languageTag)
    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(bottom = PrismaSpacing.Xxl),
    ) {
        item {
            ScreenIntroduction(
                title = tr(UiText.Settings),
                description = tr(UiText.SettingsDescription),
            )
        }
        item {
            SettingsGroup {
                SettingsRow(
                    icon = PrismaIconKind.Language,
                    title = tr(UiText.Language),
                    detail = language.autonym,
                    trailing = "${PrismaLanguages.supported.size}",
                    onClick = onLanguage,
                )
                HorizontalDivider(Modifier.padding(start = 68.dp), color = PrismaBorderSubtle)
                SettingsRow(
                    icon = PrismaIconKind.Settings,
                    title = tr(UiText.Appearance),
                    detail = "Prisma dark · OLED",
                    trailing = "AUTO",
                    onClick = {},
                )
            }
        }
        item {
            Text(
                text = tr(UiText.About),
                modifier = Modifier.padding(
                    start = PrismaSpacing.Lg,
                    top = PrismaSpacing.Xxl,
                    bottom = PrismaSpacing.Md,
                ),
                color = PrismaTextMuted,
                style = PrismaTypography.labelMedium,
            )
        }
        item {
            SettingsGroup {
                SettingsRow(
                    icon = PrismaIconKind.Translate,
                    title = "Prisma 0.0.1",
                    detail = "x86-64 → ARM64 dynamic translation",
                    trailing = "DEV",
                    onClick = {},
                )
            }
        }
    }
}

@Composable
private fun ScreenIntroduction(title: String, description: String) {
    Column(modifier = Modifier.padding(PrismaSpacing.Lg)) {
        Text(text = title, color = PrismaTextPrimary, style = PrismaTypography.headlineLarge)
        Text(
            text = description,
            modifier = Modifier.padding(top = PrismaSpacing.Xs),
            color = PrismaTextMuted,
            style = PrismaTypography.bodyLarge,
        )
    }
}

@Composable
private fun SettingsGroup(content: @Composable ColumnScope.() -> Unit) {
    Surface(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = PrismaSpacing.Lg),
        color = PrismaSurface,
        shape = RoundedCornerShape(PrismaRadii.Lg),
        border = BorderStroke(PrismaComponents.Border, PrismaBorderSubtle),
    ) {
        Column(content = content)
    }
}

@Composable
private fun SettingsRow(
    icon: PrismaIconKind,
    title: String,
    detail: String,
    trailing: String,
    onClick: () -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .height(74.dp)
            .clickable(onClick = onClick)
            .padding(horizontal = PrismaSpacing.Lg),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        PrismaGlyph(icon, PrismaPrimary, Modifier.size(22.dp))
        Column(
            modifier = Modifier
                .weight(1f)
                .padding(horizontal = PrismaSpacing.Lg),
        ) {
            Text(text = title, color = PrismaTextPrimary, style = PrismaTypography.titleMedium)
            Text(text = detail, color = PrismaTextMuted, style = PrismaTypography.bodySmall)
        }
        Text(
            text = trailing,
            color = PrismaTextMuted,
            style = PrismaTypography.labelSmall,
            fontFamily = FontFamily.Monospace,
        )
        PrismaGlyph(PrismaIconKind.Chevron, PrismaTextMuted, Modifier.padding(start = PrismaSpacing.Sm).size(16.dp))
    }
}

@Composable
private fun PrismaBottomBar(
    selected: MainDestination,
    onSelect: (MainDestination) -> Unit,
) {
    Surface(
        modifier = Modifier.fillMaxWidth(),
        color = PrismaInk950.copy(alpha = 0.98f),
        border = BorderStroke(PrismaComponents.Border, PrismaBorderSubtle),
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .navigationBarsPadding()
                .height(PrismaComponents.BottomBarHeight),
        ) {
            MainDestination.entries.forEach { destination ->
                val isSelected = selected == destination
                Column(
                    modifier = Modifier
                        .weight(1f)
                        .fillMaxHeight()
                        .semantics {
                            role = Role.Tab
                            this.selected = isSelected
                        }
                        .clickable { onSelect(destination) },
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.Center,
                ) {
                    Box(
                        modifier = Modifier
                            .size(width = 44.dp, height = 30.dp)
                            .background(
                                if (isSelected) PrismaPrimary.copy(alpha = 0.12f) else Color.Transparent,
                                RoundedCornerShape(PrismaRadii.Pill),
                            ),
                        contentAlignment = Alignment.Center,
                    ) {
                        PrismaGlyph(
                            destination.icon,
                            if (isSelected) PrismaPrimary else PrismaTextMuted,
                            Modifier.size(20.dp),
                        )
                    }
                    Text(
                        text = tr(destination.label),
                        modifier = Modifier.padding(top = PrismaSpacing.Xs),
                        color = if (isSelected) PrismaTextPrimary else PrismaTextMuted,
                        style = PrismaTypography.labelSmall,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun LanguagePicker(
    selectedTag: String,
    onDismiss: () -> Unit,
    onSelect: (String) -> Unit,
) {
    var query by remember { mutableStateOf("") }
    val languages = remember(query) {
        val needle = query.trim().lowercase()
        if (needle.isEmpty()) {
            PrismaLanguages.supported
        } else {
            PrismaLanguages.supported.filter {
                it.autonym.lowercase().contains(needle) ||
                    it.englishName.lowercase().contains(needle) ||
                    it.tag.lowercase().contains(needle)
            }
        }
    }

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        containerColor = PrismaSurface,
        shape = RoundedCornerShape(topStart = PrismaRadii.Xl, topEnd = PrismaRadii.Xl),
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .fillMaxHeight(0.9f)
                .navigationBarsPadding(),
        ) {
            Column(modifier = Modifier.padding(horizontal = PrismaSpacing.Lg)) {
                Text(
                    text = tr(UiText.Language),
                    color = PrismaTextPrimary,
                    style = PrismaTypography.headlineSmall,
                )
                Text(
                    text = tr(UiText.LanguageDescription),
                    color = PrismaTextMuted,
                    style = PrismaTypography.bodyMedium,
                )
                OutlinedTextField(
                    value = query,
                    onValueChange = { query = it },
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(top = PrismaSpacing.Lg),
                    placeholder = { Text(tr(UiText.SearchLanguages)) },
                    leadingIcon = {
                        PrismaGlyph(PrismaIconKind.Language, PrismaTextMuted, Modifier.size(20.dp))
                    },
                    singleLine = true,
                    shape = RoundedCornerShape(PrismaRadii.Md),
                    colors = OutlinedTextFieldDefaults.colors(
                        focusedBorderColor = PrismaPrimary,
                        unfocusedBorderColor = PrismaBorder,
                        cursorColor = PrismaPrimary,
                    ),
                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                    keyboardActions = KeyboardActions(onSearch = {}),
                )
            }
            Text(
                text = "${languages.size} LANGUAGES",
                modifier = Modifier.padding(PrismaSpacing.Lg),
                color = PrismaTextMuted,
                style = PrismaTypography.labelSmall,
                fontFamily = FontFamily.Monospace,
            )
            HorizontalDivider(color = PrismaBorderSubtle)
            LazyColumn(modifier = Modifier.weight(1f)) {
                items(languages, key = { it.tag }) { language ->
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .clickable { onSelect(language.tag) }
                            .padding(horizontal = PrismaSpacing.Lg, vertical = PrismaSpacing.Md),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Box(
                            modifier = Modifier
                                .size(38.dp)
                                .background(
                                    if (language.tag == selectedTag) {
                                        PrismaPrimary.copy(alpha = 0.13f)
                                    } else {
                                        PrismaSurfaceElevated
                                    },
                                    RoundedCornerShape(PrismaRadii.Md),
                                ),
                            contentAlignment = Alignment.Center,
                        ) {
                            Text(
                                text = language.tag.substringBefore('-').uppercase(),
                                color = if (language.tag == selectedTag) PrismaPrimary else PrismaTextMuted,
                                style = PrismaTypography.labelSmall,
                                fontFamily = FontFamily.Monospace,
                            )
                        }
                        Column(
                            modifier = Modifier
                                .weight(1f)
                                .padding(horizontal = PrismaSpacing.Lg),
                        ) {
                            Text(
                                text = language.autonym,
                                color = PrismaTextPrimary,
                                style = PrismaTypography.bodyLarge,
                            )
                            Text(
                                text = language.englishName,
                                color = PrismaTextMuted,
                                style = PrismaTypography.bodySmall,
                            )
                        }
                        if (language.tag == selectedTag) {
                            Text(
                                text = tr(UiText.Current),
                                color = PrismaPrimary,
                                style = PrismaTypography.labelSmall,
                            )
                        }
                    }
                    HorizontalDivider(Modifier.padding(start = 72.dp), color = PrismaBorderSubtle)
                }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ContainerActionSheet(
    container: Container,
    onDismiss: () -> Unit,
    onInstall: (Uri) -> Unit,
) {
    val copy = technicalCopy()
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        if (uri != null) onInstall(uri)
    }

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        containerColor = PrismaSurface,
        shape = RoundedCornerShape(topStart = PrismaRadii.Xl, topEnd = PrismaRadii.Xl),
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = PrismaSpacing.Xl)
                .navigationBarsPadding(),
            verticalArrangement = Arrangement.spacedBy(PrismaSpacing.Lg),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Box(
                    modifier = Modifier
                        .size(52.dp)
                        .background(PrismaPrimary.copy(alpha = 0.1f), RoundedCornerShape(PrismaRadii.Md)),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        text = container.name.take(1).uppercase(),
                        color = PrismaPrimary,
                        style = PrismaTypography.titleLarge,
                    )
                }
                Column(modifier = Modifier.padding(start = PrismaSpacing.Lg)) {
                    Text(container.name, color = PrismaTextPrimary, style = PrismaTypography.titleLarge)
                    Text(
                        container.winePrefix,
                        color = PrismaTextMuted,
                        style = PrismaTypography.bodySmall,
                        fontFamily = FontFamily.Monospace,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
            Row(modifier = Modifier.fillMaxWidth()) {
                PrismaFact("GUEST", "x86-64", Modifier.weight(1f))
                PrismaFact("TARGET", "ARM64", Modifier.weight(1f))
                PrismaFact("APPS", container.installedGames.toString(), Modifier.weight(1f))
            }
            Button(
                onClick = {
                    launcher.launch(arrayOf("application/x-msdownload", "application/octet-stream"))
                },
                modifier = Modifier
                    .fillMaxWidth()
                    .height(PrismaComponents.ActionHeight),
                colors = ButtonDefaults.buttonColors(
                    containerColor = PrismaPrimary,
                    contentColor = PrismaInk1000,
                ),
                shape = RoundedCornerShape(PrismaRadii.Md),
            ) {
                PrismaGlyph(PrismaIconKind.Import, PrismaInk1000, Modifier.size(20.dp))
                Text(tr(UiText.ImportApp), Modifier.padding(start = PrismaSpacing.Sm))
            }
            TextButton(onClick = onDismiss, modifier = Modifier.fillMaxWidth()) {
                Text(copy.cancel, color = PrismaTextSecondary)
            }
            Spacer(modifier = Modifier.height(PrismaSpacing.Sm))
        }
    }
}

@Composable
private fun CreateContainerDialog(onDismiss: () -> Unit, onCreate: (String) -> Unit) {
    val copy = technicalCopy()
    var name by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onDismiss,
        containerColor = PrismaSurface,
        shape = RoundedCornerShape(PrismaRadii.Xl),
        title = { Text(tr(UiText.NewWorkspace)) },
        text = {
            OutlinedTextField(
                value = name,
                onValueChange = { name = it },
                label = { Text(copy.name) },
                singleLine = true,
                shape = RoundedCornerShape(PrismaRadii.Md),
                colors = OutlinedTextFieldDefaults.colors(
                    focusedBorderColor = PrismaPrimary,
                    cursorColor = PrismaPrimary,
                ),
            )
        },
        confirmButton = {
            Button(
                onClick = { onCreate(name.trim()) },
                enabled = name.isNotBlank(),
                colors = ButtonDefaults.buttonColors(
                    containerColor = PrismaPrimary,
                    contentColor = PrismaInk1000,
                ),
            ) {
                Text(copy.create)
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text(copy.cancel, color = PrismaTextSecondary) }
        },
    )
}

@Composable
private fun PrototypeDialog(title: String, description: String, onDismiss: () -> Unit) {
    val copy = technicalCopy()
    AlertDialog(
        onDismissRequest = onDismiss,
        containerColor = PrismaSurface,
        shape = RoundedCornerShape(PrismaRadii.Xl),
        title = { Text(title) },
        text = { Text(description, color = PrismaTextSecondary) },
        confirmButton = {
            TextButton(onClick = onDismiss) { Text(copy.close, color = PrismaPrimary) }
        },
    )
}

private suspend fun importExecutable(
    context: android.content.Context,
    container: Container,
    uri: Uri,
    copy: TechnicalCopy,
    onRunExe: (String) -> Unit,
) {
    try {
        val prefixDir = File(context.filesDir, "wine-${container.id}/drive_c")
        check(prefixDir.exists() || prefixDir.mkdirs()) { "Could not create the container directory" }
        val executable = File(prefixDir, "program.exe")
        context.contentResolver.openInputStream(uri)?.use { source ->
            FileOutputStream(executable).use(source::copyTo)
        } ?: error("The selected file could not be opened")
        withContext(Dispatchers.Main) {
            Toast.makeText(context, "${copy.importedInto} ${container.name}.", Toast.LENGTH_SHORT).show()
            onRunExe(executable.absolutePath)
        }
    } catch (error: Exception) {
        withContext(Dispatchers.Main) {
            Toast.makeText(context, "${copy.importFailed}: ${error.message}", Toast.LENGTH_LONG).show()
        }
    }
}
