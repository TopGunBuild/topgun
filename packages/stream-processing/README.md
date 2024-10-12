# @topgunbuild/stream-processing

## Overview

`@topgunbuild/stream-processing` is a TypeScript library designed to handle in-memory data streaming and processing. It provides a robust framework for managing datasets retrieved from a database, allowing for efficient data manipulation and real-time updates. The library is particularly useful for applications that require continuous data streams and need to respond to changes in the underlying data source.

## Features

- **In-Memory Data Management**: Efficiently manage datasets in memory with support for sorting, filtering, and pagination.
- **Real-Time Updates**: Automatically update datasets in response to changes in the database.
- **Flexible Querying**: Use complex query parameters to fetch and manipulate data.
- **Customizable Sorting and Filtering**: Define custom sorting and filtering logic to suit your application's needs.

## Installation

To install the package, use npm or yarn:

```bash
npm install @topgunbuild/stream-processing
```

or

```bash
yarn add @topgunbuild/stream-processing
```

## Usage

### StreamProcessing Class

The `StreamProcessing` class is the core component of the library. It manages the main dataset and additional datasets before and after the main set, providing methods to handle data fetching, insertion, deletion
