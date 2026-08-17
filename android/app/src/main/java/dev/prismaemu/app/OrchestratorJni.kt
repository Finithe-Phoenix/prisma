package dev.prismaemu.app

object OrchestratorJni {
    val executionProbeAvailable: Boolean = runCatching {
        System.loadLibrary("prisma_android")
    }.isSuccess

    external fun runExecutionProbe(): String
    external fun runExecutable(path: String): Int
    external fun spawnTerminalProcess(callback: Any)
    external fun sendTerminalInput(input: String)
}
