package dev.prismaemu.app

import java.net.HttpURLConnection
import java.net.URL

object ExecutionProbeClient {
    private const val EMULATOR_WORKER_URL = "http://10.0.2.2:8765/probe"

    fun run(): String {
        if (OrchestratorJni.executionProbeAvailable) {
            return OrchestratorJni.runExecutionProbe()
        }

        val connection = URL(EMULATOR_WORKER_URL).openConnection() as HttpURLConnection
        return try {
            connection.requestMethod = "GET"
            connection.connectTimeout = 5_000
            connection.readTimeout = 180_000
            val status = connection.responseCode
            val stream = if (status in 200..299) connection.inputStream else connection.errorStream
            val body = stream?.bufferedReader()?.use { it.readText() }.orEmpty().trim()
            if (status !in 200..299) {
                "FAILED|stage=worker|host=x86_64|error=http-$status:${body.take(240)}"
            } else {
                body
            }
        } catch (error: Exception) {
            "UNAVAILABLE|stage=worker|host=x86_64|error=${error.javaClass.simpleName}"
        } finally {
            connection.disconnect()
        }
    }
}
