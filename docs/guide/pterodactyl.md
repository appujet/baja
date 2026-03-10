# Pterodactyl Hosting

This guide explains how to host your own Rustalink node on a Pterodactyl panel.

## Prerequisites

- A Pterodactyl panel with administrative access (to import the egg).
- A node with Docker support.

## Importing the Egg

1. Download the `egg-rustalink.json` file from <a href="/Rustalink/egg-rustalink.json" download>here</a>.
2. Log in to your Pterodactyl panel as an administrator.
3. Go to **Nests** in the admin sidebar.
4. Select a nest (e.g., "Generic") or create a new one.
5. Click the **Import Egg** button.
6. Upload the `egg-rustalink.json` file and click **Import**.

## Creating the Server

1. Go to the **Servers** section in the admin panel.
2. Click **Create New**.
3. Fill in the server details (Name, Owner, etc.).
4. In the **Nest Configuration** section, select the nest where you imported the egg and choose **Rustalink** as the egg.
5. Set the **Docker Image** to `ghcr.io/bongodevs/rustalink:latest`.
6. Configure the resource limits as needed.
7. Click **Create Server**.

## Configuration

Once the server is created, Pterodactyl will automatically generate a `config.toml` file if it doesn't exist (using the installation script).

### Environment Variables

You can configure basic settings through the **Startup** tab in the server console:

- **Server Password**: Set the `SERVER_AUTH` variable to your desired authorization token.
- **Server Port**: This is automatically linked to the server's primary allocation.

### Advanced Configuration

For more advanced settings, you can edit the `config.toml` file directly via the **File Manager**.

> [!TIP]
> Make sure the `server.address` is set to `0.0.0.0` inside `config.toml` to allow external connections (the egg handles this by default).

## Support

If you encounter any issues, feel free to open an issue on our [GitHub repository](https://github.com/bongodevs/Rustalink/issues).
