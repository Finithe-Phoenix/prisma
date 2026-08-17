package dev.prismaemu.app

import android.app.Activity
import android.os.Build
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.SideEffect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.view.WindowCompat

// Primitive tokens.
val PrismaInk1000 = Color(0xFF040608)
val PrismaInk950 = Color(0xFF080C10)
val PrismaInk900 = Color(0xFF0C1218)
val PrismaInk850 = Color(0xFF111923)
val PrismaInk800 = Color(0xFF18232F)
val PrismaInk700 = Color(0xFF223141)
val PrismaSlate500 = Color(0xFF728196)
val PrismaSlate300 = Color(0xFFA8B4C3)
val PrismaWhite = Color(0xFFF5F8FA)
val PrismaCyan400 = Color(0xFF50E6D7)
val PrismaCyan500 = Color(0xFF24CDBF)
val PrismaViolet400 = Color(0xFF9587FF)
val PrismaViolet500 = Color(0xFF7566EF)
val PrismaRose400 = Color(0xFFFF6AAE)
val PrismaGreen400 = Color(0xFF55DDA7)
val PrismaAmber400 = Color(0xFFFFC867)
val PrismaRed400 = Color(0xFFFF6B72)

// Semantic tokens.
val PrismaBackground = PrismaInk1000
val PrismaSurface = PrismaInk900
val PrismaSurfaceElevated = PrismaInk850
val PrismaSurfaceInteractive = PrismaInk800
val PrismaBorder = PrismaInk700
val PrismaBorderSubtle = PrismaInk800
val PrismaTextPrimary = PrismaWhite
val PrismaTextSecondary = PrismaSlate300
val PrismaTextMuted = PrismaSlate500
val PrismaPrimary = PrismaCyan400
val PrismaPrimaryStrong = PrismaCyan500
val PrismaSecondary = PrismaViolet400
val PrismaSecondaryStrong = PrismaViolet500
val PrismaAccent = PrismaRose400
val PrismaSuccess = PrismaGreen400
val PrismaWarning = PrismaAmber400
val PrismaError = PrismaRed400

// Compatibility aliases for renderer and terminal surfaces.
val OLEDBlack = PrismaBackground
val DarkSurface = PrismaSurface
val DarkSurfaceVariant = PrismaSurfaceElevated
val NeonCyan = PrismaPrimary
val NeonMagenta = PrismaAccent
val SoftCyan = Color(0xFFD3FBF7)
val WhiteText = PrismaTextPrimary
val GrayText = PrismaTextSecondary

object PrismaSpacing {
    val Xxs = 2.dp
    val Xs = 4.dp
    val Sm = 8.dp
    val Md = 12.dp
    val Lg = 16.dp
    val Xl = 20.dp
    val Xxl = 24.dp
    val Section = 32.dp
    val Hero = 40.dp
}

object PrismaRadii {
    val Sm = 8.dp
    val Md = 12.dp
    val Lg = 16.dp
    val Xl = 22.dp
    val Hero = 28.dp
    val Pill = 100.dp
}

object PrismaComponents {
    val ScreenPadding = PrismaSpacing.Lg
    val CardPadding = PrismaSpacing.Xl
    val ActionHeight = 56.dp
    val TouchTarget = 48.dp
    val StatusDot = 7.dp
    val Border = 1.dp
    val TopBarHeight = 68.dp
    val ListRowHeight = 76.dp
    val RowGlyph = 44.dp
    val DividerInset = 76.dp
    val BottomBarHeight = 72.dp
    val HeroHeight = 294.dp
}

object PrismaInspector {
    val Panel = PrismaInk950
    val Grid = PrismaInk800
    val Selection = PrismaPrimary.copy(alpha = 0.09f)
    val Corner = PrismaRadii.Md
    val RowHeight = 58.dp
}

private val PrismaColorScheme = darkColorScheme(
    primary = PrismaPrimary,
    onPrimary = PrismaInk1000,
    primaryContainer = Color(0xFF103D3B),
    onPrimaryContainer = SoftCyan,
    secondary = PrismaSecondary,
    onSecondary = PrismaInk1000,
    secondaryContainer = Color(0xFF29234F),
    onSecondaryContainer = Color(0xFFE4DFFF),
    tertiary = PrismaAccent,
    onTertiary = PrismaInk1000,
    background = PrismaBackground,
    onBackground = PrismaTextPrimary,
    surface = PrismaSurface,
    onSurface = PrismaTextPrimary,
    surfaceVariant = PrismaSurfaceElevated,
    onSurfaceVariant = PrismaTextSecondary,
    outline = PrismaBorder,
    error = PrismaError,
)

val PrismaTypography = Typography(
    displayLarge = TextStyle(
        fontFamily = FontFamily.SansSerif,
        fontWeight = FontWeight.Black,
        fontSize = 42.sp,
        lineHeight = 45.sp,
        letterSpacing = (-1.5).sp,
    ),
    displaySmall = TextStyle(
        fontFamily = FontFamily.SansSerif,
        fontWeight = FontWeight.ExtraBold,
        fontSize = 34.sp,
        lineHeight = 38.sp,
        letterSpacing = (-0.9).sp,
    ),
    headlineLarge = TextStyle(
        fontFamily = FontFamily.SansSerif,
        fontWeight = FontWeight.Bold,
        fontSize = 28.sp,
        lineHeight = 34.sp,
        letterSpacing = (-0.5).sp,
    ),
    headlineSmall = TextStyle(
        fontFamily = FontFamily.SansSerif,
        fontWeight = FontWeight.Bold,
        fontSize = 21.sp,
        lineHeight = 27.sp,
        letterSpacing = (-0.2).sp,
    ),
    titleLarge = TextStyle(
        fontFamily = FontFamily.SansSerif,
        fontWeight = FontWeight.SemiBold,
        fontSize = 18.sp,
        lineHeight = 24.sp,
    ),
    titleMedium = TextStyle(
        fontFamily = FontFamily.SansSerif,
        fontWeight = FontWeight.SemiBold,
        fontSize = 15.sp,
        lineHeight = 21.sp,
    ),
    bodyLarge = TextStyle(
        fontFamily = FontFamily.SansSerif,
        fontWeight = FontWeight.Normal,
        fontSize = 15.sp,
        lineHeight = 22.sp,
    ),
    bodyMedium = TextStyle(
        fontFamily = FontFamily.SansSerif,
        fontWeight = FontWeight.Normal,
        fontSize = 13.sp,
        lineHeight = 19.sp,
    ),
    bodySmall = TextStyle(
        fontFamily = FontFamily.SansSerif,
        fontWeight = FontWeight.Normal,
        fontSize = 12.sp,
        lineHeight = 17.sp,
    ),
    labelLarge = TextStyle(
        fontFamily = FontFamily.SansSerif,
        fontWeight = FontWeight.Bold,
        fontSize = 14.sp,
        lineHeight = 18.sp,
    ),
    labelMedium = TextStyle(
        fontFamily = FontFamily.SansSerif,
        fontWeight = FontWeight.SemiBold,
        fontSize = 11.sp,
        lineHeight = 15.sp,
        letterSpacing = 0.55.sp,
    ),
    labelSmall = TextStyle(
        fontFamily = FontFamily.SansSerif,
        fontWeight = FontWeight.Medium,
        fontSize = 10.sp,
        lineHeight = 14.sp,
        letterSpacing = 0.45.sp,
    ),
)

private val PrismaShapes = Shapes(
    extraSmall = RoundedCornerShape(PrismaRadii.Sm),
    small = RoundedCornerShape(PrismaRadii.Md),
    medium = RoundedCornerShape(PrismaRadii.Lg),
    large = RoundedCornerShape(PrismaRadii.Xl),
    extraLarge = RoundedCornerShape(PrismaRadii.Hero),
)

@Composable
fun PrismaTheme(
    dynamicColor: Boolean = false,
    content: @Composable () -> Unit,
) {
    val context = LocalContext.current
    val colorScheme = if (dynamicColor && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
        dynamicDarkColorScheme(context)
    } else {
        PrismaColorScheme
    }

    val view = LocalView.current
    if (!view.isInEditMode) {
        SideEffect {
            val window = (view.context as Activity).window
            window.statusBarColor = Color.Transparent.toArgb()
            window.navigationBarColor = PrismaBackground.toArgb()
            WindowCompat.setDecorFitsSystemWindows(window, false)
            WindowCompat.getInsetsController(window, view).apply {
                isAppearanceLightStatusBars = false
                isAppearanceLightNavigationBars = false
            }
        }
    }

    MaterialTheme(
        colorScheme = colorScheme,
        typography = PrismaTypography,
        shapes = PrismaShapes,
        content = content,
    )
}
