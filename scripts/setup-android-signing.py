import re

build_gradle = "src-tauri/gen/android/app/build.gradle.kts"
keystore_props = "src-tauri/gen/android/keystore.properties"
keystore_file = "src-tauri/android.keystore"

import os
github_workspace = os.environ.get("GITHUB_WORKSPACE", os.getcwd())

with open(keystore_props, "w") as f:
    f.write(f"password=android\n")
    f.write(f"keyAlias=key0\n")
    f.write(f"storeFile={github_workspace}/src-tauri/android.keystore\n")

with open(build_gradle) as f:
    content = f.read()

content = content.replace(
    "import java.util.Properties",
    "import java.util.Properties\nimport java.io.FileInputStream"
)

signing_block = (
    '    signingConfigs {\n'
    '        create("release") {\n'
    '            val keystorePropertiesFile = rootProject.file("keystore.properties")\n'
    '            val keystoreProperties = Properties()\n'
    '            if (keystorePropertiesFile.exists()) {\n'
    '                keystoreProperties.load(FileInputStream(keystorePropertiesFile))\n'
    '            }\n'
    '            keyAlias = keystoreProperties["keyAlias"] as String\n'
    '            keyPassword = keystoreProperties["password"] as String\n'
    '            storeFile = file(keystoreProperties["storeFile"] as String)\n'
    '            storePassword = keystoreProperties["password"] as String\n'
    '        }\n'
    '    }'
)

content = content.replace(
    "    buildTypes {",
    signing_block + "\n    buildTypes {"
)

content = content.replace(
    'getByName("release") {',
    'getByName("release") {\n            signingConfig = signingConfigs.getByName("release")'
)

with open(build_gradle, "w") as f:
    f.write(content)

print("Android signing configured successfully")
