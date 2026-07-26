package dev.prismaemu.app

import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.view.SurfaceHolder

object Win32Renderer {
    var surfaceHolder: SurfaceHolder? = null

    @JvmStatic
    fun drawText(text: String, x: Int, y: Int) {
        val holder = surfaceHolder ?: return
        var canvas: Canvas? = null
        try {
            canvas = holder.lockCanvas()
            if (canvas != null) {
                val paint = Paint().apply {
                    color = Color.WHITE
                    textSize = 48f
                }
                canvas.drawText(text, x.toFloat(), y.toFloat(), paint)
            }
        } finally {
            if (canvas != null) {
                holder.unlockCanvasAndPost(canvas)
            }
        }
    }

    @JvmStatic
    fun fillRect(x: Int, y: Int, width: Int, height: Int, color: Int) {
        val holder = surfaceHolder ?: return
        var canvas: Canvas? = null
        try {
            canvas = holder.lockCanvas()
            if (canvas != null) {
                val paint = Paint().apply {
                    this.color = color
                }
                canvas.drawRect(
                    x.toFloat(),
                    y.toFloat(),
                    (x + width).toFloat(),
                    (y + height).toFloat(),
                    paint
                )
            }
        } finally {
            if (canvas != null) {
                holder.unlockCanvasAndPost(canvas)
            }
        }
    }
}
