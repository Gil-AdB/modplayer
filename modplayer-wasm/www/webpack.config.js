const CopyWebpackPlugin = require("copy-webpack-plugin");
const path = require('path');
const MiniCssExtractPlugin = require('mini-css-extract-plugin');

module.exports = {
    module: {
        rules: [
            {
                test: /\.(css)$/,
                use: ['style-loader', 'css-loader'],
            },
        ],
    },
        entry:  {
        bootstrap: "./bootstrap.js",
    },
    output: {
        path: path.resolve(__dirname, "dist"),
        filename: "bootstrap.js",
    },
    mode: "development",
    plugins: [
        new CopyWebpackPlugin(['index.html']),
    ]
    // mode: "production",
    // plugins: [
    //     new CopyWebpackPlugin(['index.html']),
    //     new MiniCssExtractPlugin()
    // ]
};