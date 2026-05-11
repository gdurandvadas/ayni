plugins {
    kotlin("jvm") version "2.0.20" apply false
    id("org.jetbrains.kotlinx.kover") version "0.9.8"
    id("io.gitlab.arturbosch.detekt") version "1.23.8"
    id("info.solidsoft.pitest") version "1.19.0"
}

subprojects {
    apply(plugin = "org.jetbrains.kotlin.jvm")
    apply(plugin = "org.jetbrains.kotlinx.kover")
    apply(plugin = "io.gitlab.arturbosch.detekt")

    extensions.configure<org.jetbrains.kotlin.gradle.dsl.KotlinJvmProjectExtension> {
        jvmToolchain(21)
    }

    dependencies {
        "testImplementation"(kotlin("test"))
    }

    tasks.withType<Test>().configureEach {
        useJUnitPlatform()
    }
}
