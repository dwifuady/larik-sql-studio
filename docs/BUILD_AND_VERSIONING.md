# Build and Versioning Guide

## Building for Windows

Since this is a Tauri application, building the executable is straightforward.

### Prerequisites
Ensure all dependencies are installed:
```bash
npm install
```

### Build Command
To build the Windows executable (`.exe`) and installer (`.msi` / `setup.exe`):

```bash
npm run tauri build
```

### Output Location
After a successful build, you will find the artifacts in:
`src-tauri/target/release/bundle/nsis/`

- **.exe installer**: This is usually named `Larik SQL Studio_<version>_x64-setup.exe`. This is what you should distribute to users.
- **.msi installer**: Located in `../msi/` if configured, but NSIS is the default for Windows.

> **Note**: The bare executable inside `src-tauri/target/release/` is not portable as it may miss runtime dependencies or assets packaged by the bundler. Always use the installer for distribution.

---

## Automated Releases (CI/CD)

We have set up a GitHub Action to automatically build and release the application when you push a version tag.

### Workflow
1.  **Trigger**: Push a tag starting with `v` (e.g., `v0.3.1`).
2.  **Action**: The server builds the Windows application.
3.  **Result**: A "Draft Release" is created on GitHub with the `.exe` and `.msi` installers attached.

### How to Release
Follow the [Recommended Release Workflow](#recommended-release-workflow) below. When you run `git push origin main --tags`, the GitHub Action will start.

---

## Code Signing (Windows)

To prevent the "Unknown Publisher" warning (SmartScreen) when users install the app, the executable needs to be signed.

### 1. Purchase a Certificate (Production)
For a professional release, you need a **Code Signing Certificate**.
- **OV (Organization Validation)**: ~\$100-\$400/year. Removes "Unknown Publisher" but requires reputation buildup to bypass SmartScreen immediately.
- **EV (Extended Validation)**: ~\$300-\$600/year. Bypasses SmartScreen immediately. Requires a hardware token or cloud HSM.

**Configuration**:
If you have a certificate, add these secrets to your GitHub Repository:
- `TAURI_SIGNING_PRIVATE_KEY`: Path or content of the private key.
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: Password for the key.

The Tauri Action will automatically pick these up and sign the build.

### 2. Free Options for Open Source
There are organizations that provide free code signing for qualifying open-source projects:

-   **[SignPath Foundation](https://about.signpath.io/product/open-source)**: Offers free code signing with hardware-backed keys and CI/CD integration. Requires your project to be open source, actively maintained, and free of malware.
-   **[OSSign](https://www.ossign.org/)**: Another option for free open-source signing.

### 3. Open Source / No Certificate (Beta)
For a Beta open-source project, you can skip signing initially.
- **Consequence**: Users will see a "Windows protected your PC" popup.
- **Workaround**: Users must click **"More info" -> "Run anyway"**.
- **Mitigation**: Document this behavior in your README so users know it's expected.

---

## Versioning Strategy

### Semantic Versioning (SemVer)
We follow [Semantic Versioning](https://semver.org/).
Since the application is in **Beta**, we stick to the `0.x.x` scheme:

- **0.x.x**: Major version 0 indicates initial development. Anything MAY change at any time. The public API should not be considered stable.
- **Minor (x)**: Increment for new features or significant changes (e.g., `0.3.0` -> `0.4.0`).
- **Patch (x)**: Increment for bug fixes (e.g., `0.3.1` -> `0.3.2`).

### Branching Model
For an open-source project of this scale, a simple branching model is recommended:

1.  **`main` Branch**: Contains the stable, releasable code.
2.  **`dev` Branch (Optional)**: If you possess high velocity or multiple contributors, use a `dev` branch for integration.
3.  **Feature Branches**: Create branches for specific features or fixes (e.g., `feat/new-sidebar`, `fix/login-bug`) and merge them into `main` (or `dev`) via Pull Request.

**Releasing**:
When you are ready to release a new version:
1.  Make sure your local branch is up to date.
2.  Bump the version numbers (see below).
3.  Commit the version bump.
4.  Create a git tag (e.g., `v0.3.2`).
5.  Push the commit and tag.
6.  Run the build command (or let GitHub Actions do it).

### synchronizing Versions
Currently, version numbers exist in two places and **must match**:

1.  `package.json`
2.  `src-tauri/tauri.conf.json`

### Recommended Release Workflow

1.  **Update Version**:
    Manually update version in `package.json` and `src-tauri/tauri.conf.json`.
    
    *Alternatively, use `npm version` for package.json, but remember to update tauri.conf.json manually.*

2.  **Commit**:
    ```bash
    git add package.json src-tauri/tauri.conf.json
    git commit -m "chore(release): v0.3.2"
    ```

3.  **Tag**:
    ```bash
    git tag v0.3.2
    git push origin main --tags
    ```
    *This will trigger the GitHub Action release.*

4.  **Verify**:
    Check the "Actions" tab on GitHub. Once finished, go to "Releases", edit the draft, and publish it.
