plugins {
    kotlin("jvm") version "2.0.20"
    id("info.solidsoft.pitest") version "1.19.0"
    jacoco
}

repositories { mavenCentral() }
kotlin { jvmToolchain(21) }
dependencies { testImplementation(kotlin("test-junit")) }
jacoco { toolVersion = "0.8.12" }
tasks.test { useJUnit() }
tasks.jacocoTestReport {
    dependsOn(tasks.test)
    reports { xml.required.set(true) }
}

pitest {
    targetClasses.set(setOf("CounterKt"))
    targetTests.set(setOf("CounterTest"))
    outputFormats.set(setOf("XML"))
    timestampedReports.set(false)
}
