plugins {
    kotlin("jvm")
}

dependencies {
    implementation(project(":libs:greeting"))
    implementation(project(":libs:math"))
    implementation("com.squareup.okhttp3:okhttp:4.12.0")
}
