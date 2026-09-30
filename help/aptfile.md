# Aptfile

A file at the root of your repository listing the Ubuntu packages your app needs on the server, one per line.

```
ffmpeg
libvips42
# chromium for PDF rendering
chromium-browser
```

- A package name matches `^[a-z0-9][a-z0-9+._-]*$`. Anything else on a line is ignored and named in the deploy log.
- Every deploy installs what the tag's Aptfile adds before the build runs.
- A package that disappears from the Aptfile is **not** uninstalled by the deploy. The deploy log says it was kept, the app's page shows a notice with an Uninstall button, and Ferrum only removes it when no other app lists it and the server did not have it before Ferrum.
- Packages are system-wide. Two apps needing conflicting versions of the same library will collide.

The panel's System packages list on the Configuration tab is the same list; the Aptfile adds to it on each deploy.
