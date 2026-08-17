package dev.prismaemu.app

import org.junit.Test

class DemoManifestTest {
    @Test
    fun demoManifestKeepsExecutionClaimsHonest() {
        DemoManifest.validate()
    }
}
