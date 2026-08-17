package dev.prismaemu.app

import org.junit.Test

class LocalizationTest {
    @Test
    fun `catalog has more than fifty complete unique language packs`() {
        PrismaLanguages.validate()
    }
}
