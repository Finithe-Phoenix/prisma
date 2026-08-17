package dev.prismaemu.app

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

enum class PrismaIconKind {
    Home,
    Library,
    Activity,
    Settings,
    Import,
    Translate,
    Terminal,
    Controller,
    Language,
    Chevron,
}

@Composable
fun PrismaMark(modifier: Modifier = Modifier) {
    Canvas(modifier = modifier.size(32.dp)) {
        val stroke = 4.dp.toPx()
        drawLine(
            color = PrismaPrimary,
            start = Offset(size.width * 0.18f, size.height * 0.78f),
            end = Offset(size.width * 0.48f, size.height * 0.18f),
            strokeWidth = stroke,
            cap = StrokeCap.Round,
        )
        drawLine(
            color = PrismaSecondary,
            start = Offset(size.width * 0.48f, size.height * 0.18f),
            end = Offset(size.width * 0.8f, size.height * 0.78f),
            strokeWidth = stroke,
            cap = StrokeCap.Round,
        )
        drawLine(
            color = PrismaAccent,
            start = Offset(size.width * 0.8f, size.height * 0.78f),
            end = Offset(size.width * 0.18f, size.height * 0.78f),
            strokeWidth = stroke,
            cap = StrokeCap.Round,
        )
    }
}

@Composable
fun PrismaGlyph(
    kind: PrismaIconKind,
    tint: Color,
    modifier: Modifier = Modifier,
) {
    Canvas(modifier = modifier.size(24.dp)) {
        val stroke = 1.8.dp.toPx()
        val left = size.width * 0.18f
        val right = size.width * 0.82f
        val top = size.height * 0.18f
        val bottom = size.height * 0.82f
        when (kind) {
            PrismaIconKind.Home -> {
                drawLine(tint, Offset(left, size.height * 0.48f), Offset(size.width / 2, top), stroke, cap = StrokeCap.Round)
                drawLine(tint, Offset(size.width / 2, top), Offset(right, size.height * 0.48f), stroke, cap = StrokeCap.Round)
                drawRoundRect(
                    tint,
                    topLeft = Offset(size.width * 0.27f, size.height * 0.43f),
                    size = androidx.compose.ui.geometry.Size(size.width * 0.46f, size.height * 0.38f),
                    cornerRadius = CornerRadius(2.dp.toPx()),
                    style = Stroke(stroke),
                )
            }
            PrismaIconKind.Library -> {
                repeat(3) { index ->
                    val y = size.height * (0.25f + index * 0.25f)
                    drawCircle(tint, radius = 1.6.dp.toPx(), center = Offset(left, y))
                    drawLine(tint, Offset(size.width * 0.34f, y), Offset(right, y), stroke, cap = StrokeCap.Round)
                }
            }
            PrismaIconKind.Activity -> {
                val points = listOf(
                    Offset(left, size.height * 0.62f),
                    Offset(size.width * 0.36f, size.height * 0.62f),
                    Offset(size.width * 0.47f, size.height * 0.28f),
                    Offset(size.width * 0.6f, size.height * 0.75f),
                    Offset(right, size.height * 0.42f),
                )
                points.zipWithNext().forEach { (start, end) ->
                    drawLine(tint, start, end, stroke, cap = StrokeCap.Round)
                }
            }
            PrismaIconKind.Settings -> {
                drawCircle(tint, radius = size.minDimension * 0.27f, center = center, style = Stroke(stroke))
                drawCircle(tint, radius = size.minDimension * 0.08f, center = center, style = Stroke(stroke))
                repeat(4) { index ->
                    val angle = index * Math.PI / 2
                    drawLine(
                        tint,
                        Offset(
                            center.x + kotlin.math.cos(angle).toFloat() * size.width * 0.31f,
                            center.y + kotlin.math.sin(angle).toFloat() * size.height * 0.31f,
                        ),
                        Offset(
                            center.x + kotlin.math.cos(angle).toFloat() * size.width * 0.4f,
                            center.y + kotlin.math.sin(angle).toFloat() * size.height * 0.4f,
                        ),
                        stroke,
                        cap = StrokeCap.Round,
                    )
                }
            }
            PrismaIconKind.Import -> {
                drawLine(tint, Offset(size.width / 2, top), Offset(size.width / 2, size.height * 0.62f), stroke, cap = StrokeCap.Round)
                drawLine(tint, Offset(size.width * 0.36f, size.height * 0.48f), Offset(size.width / 2, size.height * 0.62f), stroke, cap = StrokeCap.Round)
                drawLine(tint, Offset(size.width * 0.64f, size.height * 0.48f), Offset(size.width / 2, size.height * 0.62f), stroke, cap = StrokeCap.Round)
                drawLine(tint, Offset(left, bottom), Offset(right, bottom), stroke, cap = StrokeCap.Round)
            }
            PrismaIconKind.Translate -> {
                drawRoundRect(tint, Offset(left, top), androidx.compose.ui.geometry.Size(size.width * 0.64f, size.height * 0.64f), CornerRadius(4.dp.toPx()), style = Stroke(stroke))
                drawLine(tint, Offset(size.width * 0.3f, size.height * 0.42f), Offset(size.width * 0.7f, size.height * 0.42f), stroke)
                drawLine(tint, Offset(size.width * 0.5f, size.height * 0.3f), Offset(size.width * 0.5f, size.height * 0.67f), stroke)
            }
            PrismaIconKind.Terminal -> {
                drawLine(tint, Offset(left, size.height * 0.34f), Offset(size.width * 0.42f, size.height * 0.5f), stroke, cap = StrokeCap.Round)
                drawLine(tint, Offset(size.width * 0.42f, size.height * 0.5f), Offset(left, size.height * 0.66f), stroke, cap = StrokeCap.Round)
                drawLine(tint, Offset(size.width * 0.52f, size.height * 0.7f), Offset(right, size.height * 0.7f), stroke, cap = StrokeCap.Round)
            }
            PrismaIconKind.Controller -> {
                drawRoundRect(tint, Offset(left, size.height * 0.3f), androidx.compose.ui.geometry.Size(size.width * 0.64f, size.height * 0.4f), CornerRadius(7.dp.toPx()), style = Stroke(stroke))
                drawLine(tint, Offset(size.width * 0.34f, size.height * 0.42f), Offset(size.width * 0.34f, size.height * 0.58f), stroke)
                drawLine(tint, Offset(size.width * 0.27f, size.height * 0.5f), Offset(size.width * 0.41f, size.height * 0.5f), stroke)
                drawCircle(tint, 1.5.dp.toPx(), Offset(size.width * 0.68f, size.height * 0.46f))
                drawCircle(tint, 1.5.dp.toPx(), Offset(size.width * 0.75f, size.height * 0.56f))
            }
            PrismaIconKind.Language -> {
                drawCircle(tint, size.minDimension * 0.31f, center, style = Stroke(stroke))
                drawOval(tint, Offset(size.width * 0.36f, top), androidx.compose.ui.geometry.Size(size.width * 0.28f, size.height * 0.64f), style = Stroke(stroke))
                drawLine(tint, Offset(left, center.y), Offset(right, center.y), stroke)
            }
            PrismaIconKind.Chevron -> {
                drawLine(tint, Offset(size.width * 0.38f, top), Offset(size.width * 0.64f, center.y), stroke, cap = StrokeCap.Round)
                drawLine(tint, Offset(size.width * 0.64f, center.y), Offset(size.width * 0.38f, bottom), stroke, cap = StrokeCap.Round)
            }
        }
    }
}

@Composable
fun PrismaTopBar(
    title: String,
    subtitle: String,
    statusColor: Color,
    onBack: (() -> Unit)? = null,
    actions: @Composable RowScope.() -> Unit = {},
) {
    val copy = technicalCopy()
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .statusBarsPadding()
            .height(PrismaComponents.TopBarHeight)
            .padding(horizontal = PrismaSpacing.Sm),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (onBack != null) {
            IconButton(onClick = onBack) {
                Icon(
                    imageVector = Icons.AutoMirrored.Filled.ArrowBack,
                    contentDescription = copy.back,
                    tint = PrismaTextPrimary,
                )
            }
        } else {
            Box(modifier = Modifier.width(PrismaSpacing.Sm))
        }
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = title,
                color = PrismaTextPrimary,
                style = PrismaTypography.titleLarge,
                fontWeight = FontWeight.SemiBold,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Row(verticalAlignment = Alignment.CenterVertically) {
                Box(
                    modifier = Modifier
                        .size(PrismaComponents.StatusDot)
                        .background(statusColor, CircleShape),
                )
                Box(modifier = Modifier.width(PrismaSpacing.Sm))
                Text(
                    text = subtitle,
                    color = PrismaTextMuted,
                    style = PrismaTypography.labelMedium,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
        Row(content = actions, verticalAlignment = Alignment.CenterVertically)
    }
    HorizontalDivider(color = PrismaBorderSubtle)
}

@Composable
fun PrismaSectionHeader(
    title: String,
    detail: String? = null,
    actionLabel: String? = null,
    onAction: (() -> Unit)? = null,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(
                start = PrismaSpacing.Lg,
                end = PrismaSpacing.Sm,
                top = PrismaSpacing.Xxl,
                bottom = PrismaSpacing.Md,
            ),
        verticalAlignment = Alignment.Bottom,
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = title,
                color = PrismaTextPrimary,
                style = PrismaTypography.titleLarge,
                fontWeight = FontWeight.SemiBold,
            )
            if (detail != null) {
                Text(text = detail, color = PrismaTextMuted, style = PrismaTypography.bodySmall)
            }
        }
        if (actionLabel != null && onAction != null) {
            TextButton(onClick = onAction) {
                Text(actionLabel, color = PrismaPrimary)
            }
        }
    }
}

@Composable
fun PrismaUtilityRow(
    code: String,
    title: String,
    detail: String,
    trailing: String,
    onClick: () -> Unit,
    enabled: Boolean = true,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .height(PrismaComponents.ListRowHeight)
            .clickable(enabled = enabled, onClick = onClick)
            .padding(horizontal = PrismaSpacing.Lg),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(PrismaSpacing.Lg),
    ) {
        Box(
            modifier = Modifier
                .size(PrismaComponents.RowGlyph)
                .background(PrismaInspector.Selection, RoundedCornerShape(PrismaRadii.Md)),
            contentAlignment = Alignment.Center,
        ) {
            Text(
                text = code,
                color = PrismaPrimary,
                style = PrismaTypography.labelMedium,
                fontFamily = FontFamily.Monospace,
                fontWeight = FontWeight.SemiBold,
            )
        }
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = title,
                color = if (enabled) PrismaTextPrimary else PrismaTextMuted,
                style = PrismaTypography.bodyLarge,
                fontWeight = FontWeight.Medium,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                text = detail,
                color = PrismaTextMuted,
                style = PrismaTypography.bodySmall,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        Text(
            text = trailing,
            color = PrismaTextMuted,
            style = PrismaTypography.labelMedium,
            fontFamily = FontFamily.Monospace,
        )
    }
}

@Composable
fun PrismaInsetDivider() {
    HorizontalDivider(
        modifier = Modifier.padding(start = PrismaComponents.DividerInset),
        color = PrismaBorderSubtle,
    )
}

@Composable
fun PrismaFact(label: String, value: String, modifier: Modifier = Modifier) {
    Column(modifier = modifier) {
        Text(text = label, color = PrismaTextMuted, style = PrismaTypography.labelSmall)
        Text(
            text = value,
            color = PrismaTextPrimary,
            style = PrismaTypography.bodySmall,
            fontFamily = FontFamily.Monospace,
            fontWeight = FontWeight.Medium,
        )
    }
}
